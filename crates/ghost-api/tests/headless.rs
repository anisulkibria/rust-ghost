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
        Self::with_host_settings(json!(null)).await
    }

    /// The same fixture with `hostSettings` filled in, for the limit checks:
    /// on a self-hosted install no limit is configured and there is nothing
    /// to test, so the only way to exercise them is to be a host.
    async fn with_host_settings(host_settings: Value) -> Self {
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
            "database": {"connection": {"filename": ":memory:"}}, "paths": {},
            "hostSettings": host_settings,
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
            limits: Arc::new(ghost_db::limits::Limits::load(&host_settings)),
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

#[tokio::test]
async fn newsletter_opens_are_recorded_once_per_recipient_and_only_for_signed_requests() {
    use sha2::{Digest,Sha256};
    let f=Fixture::new().await;
    let mut state=f.state.clone();
    state.settings=Arc::new(state.settings.with_overrides([("email_track_opens".into(),json!(true)),("members_email_auth_secret".into(),json!("fixture-open-secret"))]));
    sqlx::query("INSERT INTO members (id,uuid,transient_id,email,status,created_at) VALUES ('open-member','open-uuid','open-transient','reader@example.test','free',datetime('now'))").execute(&state.db).await.unwrap();
    sqlx::query("INSERT INTO emails (id,post_id,status,email_count,created_at) VALUES ('open-email','open-post','submitting',1,datetime('now'))").execute(&state.db).await.unwrap();
    ghost_api::newsletter_events::record_recipient(&state,ghost_api::newsletter_events::RecipientRecord{
        email_id:"open-email",batch_id:None,member_id:"open-member",member_uuid:"open-uuid",
        email:"reader@example.test",provider_id:Some("<provider-id@mg.example>"),error:None,
    }).await.unwrap();
    // The send is what makes the member's open rate computable at all.
    let counts=|state:AppState| async move {
        let recipient:(String,Option<String>,Option<String>)=sqlx::query_as("SELECT status,delivered_at,opened_at FROM email_recipients WHERE email_id='open-email'").fetch_one(&state.db).await.unwrap();
        let opened:i64=sqlx::query_scalar("SELECT opened_count FROM emails WHERE id='open-email'").fetch_one(&state.db).await.unwrap();
        let member:(i64,i64,Option<i64>)=sqlx::query_as("SELECT email_count,email_opened_count,email_open_rate FROM members WHERE id='open-member'").fetch_one(&state.db).await.unwrap();
        (recipient,opened,member)
    };
    let (recipient,opened,member)=counts(state.clone()).await;
    assert_eq!(recipient.0,"sent");assert!(recipient.1.is_some());assert!(recipient.2.is_none());
    assert_eq!((opened,member),(0,(1,0,None)));
    // The provider id is stored unbracketed, the way Mailgun reports it back.
    let provider:String=sqlx::query_scalar("SELECT mailgun_message_id FROM email_recipients WHERE email_id='open-email'").fetch_one(&state.db).await.unwrap();
    assert_eq!(provider,"provider-id@mg.example");

    // The pixel is signed with the same uuid + HMAC pair the unsubscribe link
    // carries; an unsigned or wrongly signed request records nothing.
    let secret=b"fixture-open-secret";let mut padded=[0u8;64];padded[..secret.len()].copy_from_slice(secret);
    let mut inner=Sha256::new();inner.update(padded.map(|b|b^0x36));inner.update("open-uuid");
    let mut outer=Sha256::new();outer.update(padded.map(|b|b^0x5c));outer.update(inner.finalize());
    let signature=hex::encode(outer.finalize());
    let app=ghost_api::newsletter_events::public_router().with_state(state.clone());
    let pixel=|uri:String| {
        let app=app.clone();
        async move { app.oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap()).await.unwrap() }
    };
    for uri in ["/r/o/open-email/".to_string(), "/r/o/open-email/?m=open-uuid".to_string(), format!("/r/o/open-email/?m=open-uuid&k={}","0".repeat(64))] {
        let response=pixel(uri.clone()).await;
        // Always an image: a broken icon mid-newsletter is worse than a lost stat.
        assert_eq!(response.status(),StatusCode::OK,"{uri}");
        assert_eq!(response.headers()["content-type"],"image/gif");
        assert_eq!(counts(state.clone()).await.1,0,"{uri} recorded an open it should not have");
    }
    let uri=format!("/r/o/open-email/?m=open-uuid&k={signature}");
    for replay in [false,true] {
        let response=pixel(uri.clone()).await;
        assert_eq!(response.status(),StatusCode::OK);
        let body=to_bytes(response.into_body(),1_000).await.unwrap();
        assert_eq!(&body[..6],b"GIF89a");
        let (recipient,opened,member)=counts(state.clone()).await;
        // Counted on the first fetch and not again: a client that loads the
        // image on every scroll must not move the numbers.
        assert!(recipient.2.is_some(),"first open was not stored");
        assert_eq!((opened,member),(1,(1,1,Some(100))),"replay={replay} double-counted");
    }

    // The same counter, reached from Mailgun's own open event. Matched to the
    // recipient by the provider id, and guarded against the pixel's count.
    let mut state=state.clone();
    state.settings=Arc::new(state.settings.with_overrides([("mailgun_webhook_signing_key".into(),json!("fixture-signing-key"))]));
    let app=ghost_api::newsletter_events::public_router().with_state(state.clone());
    let timestamp=chrono::Utc::now().timestamp().to_string();let token="open-token";
    let mut padded=[0u8;64];padded[..19].copy_from_slice(b"fixture-signing-key");
    let mut inner=Sha256::new();inner.update(padded.map(|b|b^0x36));inner.update(format!("{timestamp}{token}"));
    let mut outer=Sha256::new();outer.update(padded.map(|b|b^0x5c));outer.update(inner.finalize());let webhook_signature=hex::encode(outer.finalize());
    let event=|id:&str,kind:&str| json!({"signature":{"timestamp":timestamp,"token":token,"signature":webhook_signature},
        "event-data":{"id":id,"event":kind,"recipient":"reader@example.test","message":{"headers":{"message-id":"provider-id@mg.example"}}}});
    let response=app.clone().oneshot(Request::builder().method("POST").uri("/members/webhooks/mailgun/").header("content-type","application/json").body(Body::from(event("mg-open","opened").to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(),StatusCode::OK);
    assert_eq!(counts(state.clone()).await.1,1,"the provider event double-counted an open the pixel already had");

    // A delivery event for a recipient the provider has not confirmed yet.
    sqlx::query("UPDATE email_recipients SET delivered_at=NULL WHERE email_id='open-email'").execute(&state.db).await.unwrap();
    let response=app.oneshot(Request::builder().method("POST").uri("/members/webhooks/mailgun/").header("content-type","application/json").body(Body::from(event("mg-delivered","delivered").to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(),StatusCode::OK);
    assert!(counts(state.clone()).await.0.1.is_some(),"the delivery event was not recorded");
}

#[tokio::test]
async fn the_open_pixel_goes_inside_the_body_and_only_when_tracking_is_on() {
    let f=Fixture::new().await;
    let off=ghost_api::newsletter_events::track_opens(&f.state,"an-email","<html><body><p>Hi</p></body></html>");
    assert_eq!(off,"<html><body><p>Hi</p></body></html>","tracking is off by default, so nothing is added");
    let mut state=f.state.clone();
    state.settings=Arc::new(state.settings.with_overrides([("email_track_opens".into(),json!(true))]));
    let on=ghost_api::newsletter_events::track_opens(&state,"an-email","<html><body><p>Hi</p></body></html>");
    assert!(on.contains(r#"<p>Hi</p><img src="https://example.test/r/o/an-email/?m=%%{uuid}%%&amp;k=%%{key}%%" width="1" height="1""#),"{on}");
    assert!(on.ends_with("</body></html>"),"{on}");
    // No body element to sit inside: appended rather than dropped.
    assert!(ghost_api::newsletter_events::track_opens(&state,"an-email","<p>Fragment</p>").starts_with("<p>Fragment</p><img src="),"fragment");
}

#[tokio::test]
async fn a_lexical_only_post_renders_its_media_and_layout_cards() {
    let f=Fixture::new().await;
    // A post created through the Admin API with no `html` is the case that
    // reaches the Rust renderer: there is no browser Koenig to supply it.
    let lexical=json!({"root":{"type":"root","version":1,"direction":"ltr","format":"","indent":0,"children":[
        {"type":"audio","version":1,"src":"/content/media/track.mp3","title":"Episode 1","duration":214},
        {"type":"video","version":1,"src":"/content/media/clip.mp4","thumbnailSrc":"/t.jpg","width":1920,"height":1080,"duration":42},
        {"type":"product","version":1,"productTitle":"A product","productDescription":"<p>Good</p>","productButtonEnabled":true,"productButton":"Buy","productUrl":"https://example.test/buy"},
        {"type":"header","version":2,"header":"A header","subheader":"A subheader","layout":"full","backgroundColor":"#000000"},
        {"type":"signup","version":1,"header":"Join","layout":"wide","backgroundColor":"accent"},
        {"type":"call-to-action","version":1,"layout":"minimal","textValue":"<p>Sponsored</p>","showButton":true,"buttonText":"Go","buttonUrl":"https://example.test/ad"},
        {"type":"paragraph","version":1,"children":[{"type":"tk","version":1,"text":"TK","format":0,"detail":0,"mode":"normal","style":""},{"type":"text","version":1,"text":" follow up","format":0,"detail":0,"mode":"normal","style":""}]}
    ]}}).to_string();
    let (status,created)=f.request("POST","/admin/posts/?formats=html,lexical",json!({"posts":[{"title":"Every card","lexical":lexical,"status":"published"}]}),None).await;
    assert_eq!(status,StatusCode::CREATED,"{created}");
    let html=created["posts"][0]["html"].as_str().unwrap();
    // Each of these used to render as nothing at all.
    for class in ["kg-audio-card","kg-video-card","kg-product-card","kg-header-card kg-v2","kg-signup-card","kg-cta-card"] {
        assert!(html.contains(class),"{class} is missing from {html}");
    }
    // The players carry the hooks the theme's cards.min.js binds to.
    assert!(html.contains(r#"<audio src="/content/media/track.mp3" preload="metadata">"#),"{html}");
    assert!(html.contains("kg-video-seek-slider"),"{html}");
    // Portal's signup form, and the editor's TK marker as plain text.
    assert!(html.contains(r#"data-members-form="signup""#),"{html}");
    assert!(html.contains("<p>TK follow up</p>"),"{html}");
    // The stored column is what the frontend serves, so it has them too.
    let stored:String=sqlx::query_scalar("SELECT html FROM posts WHERE id=?").bind(created["posts"][0]["id"].as_str().unwrap()).fetch_one(&f.state.db).await.unwrap();
    assert_eq!(stored,html);
}

#[tokio::test]
async fn the_emails_family_reports_status_batches_and_who_was_missed() {
    let f=Fixture::new().await;
    sqlx::query("INSERT INTO members (id,uuid,transient_id,email,name,status,created_at) VALUES ('m1','u1','t1','bounced@example.test','Bo','free',datetime('now'))").execute(&f.state.db).await.unwrap();
    sqlx::query("INSERT INTO emails (id,uuid,post_id,status,email_count,candidate_count,preflight_email_count,delivered_count,failed_count,opened_count,subject,source_type,created_at) \
                 VALUES ('e1','e-uuid','p1','submitted',3,4,3,2,1,1,'Issue one','html',datetime('now'))").execute(&f.state.db).await.unwrap();
    for (id,status,segment,count,submitted) in [("b1","submitted","status:free access:true named:true",2,2),("b2","failed","status:free access:true named:false",1,0)] {
        sqlx::query("INSERT INTO email_batches (id,email_id,status,member_segment,recipient_count,submitted_count,created_at) VALUES (?,'e1',?,?,?,?,datetime('now'))")
            .bind(id).bind(status).bind(segment).bind(count).bind(submitted).execute(&f.state.db).await.unwrap();
    }
    sqlx::query("INSERT INTO email_recipient_failures (id,email_id,batch_id,member_id,message,severity,code,failed_at) VALUES ('f1','e1','b2','m1','550 no such mailbox','permanent',550,datetime('now'))")
        .execute(&f.state.db).await.unwrap();

    let (status,list)=f.request("GET","/admin/emails/",Value::Null,None).await;
    assert_eq!(status,StatusCode::OK,"{list}");
    assert_eq!(list["emails"][0]["id"],"e1");
    // The counts Ghost keeps, including the ones that say how the audience
    // narrowed between being chosen and being sent to.
    assert_eq!(list["emails"][0]["candidate_count"],4);
    assert_eq!(list["emails"][0]["preflight_email_count"],3);
    assert_eq!(list["emails"][0]["source_type"],"html");

    let (_,read)=f.request("GET","/admin/emails/e1/",Value::Null,None).await;
    assert_eq!(read["emails"][0]["subject"],"Issue one");
    assert_eq!(f.request("GET","/admin/emails/nope/",Value::Null,None).await.0,StatusCode::NOT_FOUND);

    let (_,batches)=f.request("GET","/admin/emails/e1/batches/",Value::Null,None).await;
    assert_eq!(batches["batches"].as_array().unwrap().len(),2);
    assert_eq!(batches["batches"][0]["member_segment"],"status:free access:true named:true");
    assert_eq!(batches["batches"][1]["submitted_count"],0);

    // The question this answers is "which address bounced", so the member is
    // joined in rather than left as an id.
    let (_,failures)=f.request("GET","/admin/emails/e1/recipient-failures/",Value::Null,None).await;
    assert_eq!(failures["failures"][0]["member_email"],"bounced@example.test");
    assert_eq!(failures["failures"][0]["member_name"],"Bo");
    assert_eq!(failures["failures"][0]["severity"],"permanent");
    assert_eq!(failures["failures"][0]["code"],550);

    let (status,progress)=f.request("GET","/admin/emails/e1/status/",Value::Null,None).await;
    assert_eq!(status,StatusCode::OK,"{progress}");
    assert_eq!(progress["emails"][0]["status"],"submitted");
    assert_eq!(progress["emails"][0]["batch_count"],2);
    assert_eq!(progress["emails"][0]["submitted_batch_count"],1);

    // A contributor has no business reading the mailing list's bounces.
    let (_,cookie)=f.staff("Contributor","emails-contributor").await;
    for path in ["/admin/emails/","/admin/emails/e1/","/admin/emails/e1/batches/","/admin/emails/e1/recipient-failures/","/admin/emails/e1/status/"] {
        let response=f.app.clone().oneshot(Request::builder().uri(path).header("cookie",&cookie).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(),StatusCode::FORBIDDEN,"{path}");
    }
}

#[tokio::test]
async fn a_provider_failure_event_records_who_was_missed_and_corrects_the_counts() {
    use sha2::{Digest,Sha256};
    let f=Fixture::new().await;
    let mut state=f.state.clone();
    state.settings=Arc::new(state.settings.with_overrides([("mailgun_webhook_signing_key".into(),json!("fixture-signing-key"))]));
    sqlx::query("INSERT INTO members (id,uuid,transient_id,email,status,created_at) VALUES ('fm','fu','ft','gone@example.test','free',datetime('now'))").execute(&state.db).await.unwrap();
    sqlx::query("INSERT INTO emails (id,post_id,status,email_count,delivered_count,created_at) VALUES ('fe','fp','submitted',1,1,datetime('now'))").execute(&state.db).await.unwrap();
    ghost_api::newsletter_events::record_recipient(&state,ghost_api::newsletter_events::RecipientRecord{
        email_id:"fe",batch_id:None,member_id:"fm",member_uuid:"fu",email:"gone@example.test",
        provider_id:Some("fe.digest@mg.example"),error:None,
    }).await.unwrap();

    let app=ghost_api::newsletter_events::public_router().with_state(state.clone());
    let timestamp=chrono::Utc::now().timestamp().to_string();let token="fail-token";
    let mut padded=[0u8;64];padded[..19].copy_from_slice(b"fixture-signing-key");
    let mut inner=Sha256::new();inner.update(padded.map(|b|b^0x36));inner.update(format!("{timestamp}{token}"));
    let mut outer=Sha256::new();outer.update(padded.map(|b|b^0x5c));outer.update(inner.finalize());let signature=hex::encode(outer.finalize());
    let event=json!({"signature":{"timestamp":timestamp,"token":token,"signature":signature},
        "event-data":{"id":"mg-fail","event":"failed","severity":"permanent","recipient":"gone@example.test",
            "message":{"headers":{"message-id":"fe.digest@mg.example"}},
            "delivery-status":{"code":"550","message":"mailbox unavailable","enhanced-code":"5.1.1"}}});
    for replay in [false,true] {
        let response=app.clone().oneshot(Request::builder().method("POST").uri("/members/webhooks/mailgun/").header("content-type","application/json").body(Body::from(event.to_string())).unwrap()).await.unwrap();
        assert_eq!(response.status(),StatusCode::OK,"replay={replay}");
    }
    // Recorded once, however many times the provider redelivers the event.
    let failures:Vec<(String,Option<i64>,Option<String>,Option<String>)>=sqlx::query_as(
        "SELECT message,code,enhanced_code,severity FROM email_recipient_failures WHERE email_id='fe'",
    ).fetch_all(&state.db).await.unwrap();
    assert_eq!(failures.len(),1,"{failures:?}");
    assert_eq!(failures[0].0,"mailbox unavailable");
    assert_eq!(failures[0].1,Some(550));
    assert_eq!(failures[0].2.as_deref(),Some("5.1.1"));
    assert_eq!(failures[0].3.as_deref(),Some("permanent"));
    // A message the provider accepted and then could not deliver is no
    // longer a delivery, so the counts move back.
    let recipient:(String,Option<String>)=sqlx::query_as("SELECT status,error FROM email_recipients WHERE email_id='fe'").fetch_one(&state.db).await.unwrap();
    assert_eq!(recipient.0,"failed");
    assert_eq!(recipient.1.as_deref(),Some("mailbox unavailable"));
    let counts:(i64,i64)=sqlx::query_as("SELECT delivered_count,failed_count FROM emails WHERE id='fe'").fetch_one(&state.db).await.unwrap();
    assert_eq!(counts,(0,1));
}

#[tokio::test]
async fn a_signup_is_attributed_to_the_post_it_came_from() {
    let f=Fixture::new().await;
    let (_,created)=f.request("POST","/admin/posts/",json!({"posts":[{"title":"The post that converted","slug":"converter","status":"published"}]}),None).await;
    let post_id=created["posts"][0]["id"].as_str().unwrap().to_string();
    let (_,tag)=f.request("POST","/admin/tags/",json!({"tags":[{"name":"Topic","slug":"topic"}]}),None).await;
    let tag_id=tag["tags"][0]["id"].as_str().unwrap().to_string();

    // Portal's url history: oldest first, with the referrer on the entry
    // that came in from outside.
    let history=json!([
        {"path":"/?utm_source=newsletter&utm_medium=email&utm_campaign=launch","referrerSource":"Google","referrerMedium":"search","referrerUrl":"https://www.google.com/"},
        {"path":"/tag/topic/"},
        {"path":"/converter/"}
    ]);
    let attribution=ghost_api::member_events::attribution_from_history(&f.state,&history).await;
    // The most recent resolvable page is the one credited — the last thing
    // they were reading when they signed up.
    assert_eq!(attribution.kind.as_deref(),Some("post"));
    assert_eq!(attribution.id.as_deref(),Some(post_id.as_str()));
    assert_eq!(attribution.url.as_deref(),Some("/converter/"));
    // The referrer is the earliest one named: the thing that first sent them.
    assert_eq!(attribution.referrer_source.as_deref(),Some("Google"));
    assert_eq!(attribution.referrer_medium.as_deref(),Some("search"));
    // Campaign tags come off the URL they arrived on.
    assert_eq!(attribution.utm_source.as_deref(),Some("newsletter"));
    assert_eq!(attribution.utm_campaign.as_deref(),Some("launch"));

    // A tag archive is a resource too, when it is the last page they saw.
    let tagged=ghost_api::member_events::attribution_from_history(&f.state,&json!([{"path":"/tag/topic/"}])).await;
    assert_eq!((tagged.kind.as_deref(),tagged.id.as_deref()),(Some("tag"),Some(tag_id.as_str())));
    // A page that resolves to nothing creditable still answers the question.
    let home=ghost_api::member_events::attribution_from_history(&f.state,&json!([{"path":"/"}])).await;
    assert_eq!(home.kind.as_deref(),Some("url"));
    assert_eq!(home.id,None);
    // Nothing at all is not an error.
    assert_eq!(ghost_api::member_events::attribution_from_history(&f.state,&Value::Null).await.kind,None);

    // And the recorded signup carries all of it, which is what makes the
    // post's own analytics answerable.
    sqlx::query("INSERT INTO members (id,uuid,transient_id,email,status,created_at) VALUES ('am','au','at','attributed@example.test','free',datetime('now'))").execute(&f.state.db).await.unwrap();
    ghost_api::member_events::record_signup(&f.state,"am","member",&attribution).await.unwrap();
    let row:(Option<String>,Option<String>,Option<String>,Option<String>,String)=sqlx::query_as(
        "SELECT attribution_id,attribution_type,referrer_source,utm_campaign,source FROM members_created_events WHERE member_id='am'",
    ).fetch_one(&f.state.db).await.unwrap();
    assert_eq!(row,(Some(post_id.clone()),Some("post".into()),Some("Google".into()),Some("launch".into()),"member".into()));

    let (status,stats)=f.request("GET",&format!("/admin/stats/posts/{post_id}/growth/"),Value::Null,None).await;
    assert_eq!(status,StatusCode::OK,"{stats}");
    assert_eq!(stats["stats"][0]["free_members"],1);
    assert_eq!(stats["stats"][0]["paid_members"],0);

    let (_,top)=f.request("GET","/admin/stats/top-posts/?days=30",Value::Null,None).await;
    let found=top["stats"].as_array().unwrap().iter().find(|p|p["post_id"]==post_id.as_str()).unwrap().clone();
    assert_eq!(found["free_members"],1);

    let (_,counts)=f.request("POST","/admin/stats/posts-member-counts/",json!({"ids":[post_id]}),None).await;
    assert_eq!(counts["stats"][0][&post_id]["free"],1);
    assert_eq!(counts["stats"][0][&post_id]["total"],1);

    let (_,sources)=f.request("GET","/admin/stats/top-sources-growth/?days=30",Value::Null,None).await;
    assert_eq!(sources["stats"][0]["source"],"Google");
    assert_eq!(sources["stats"][0]["free_members"],1);
}

#[tokio::test]
async fn the_member_activity_feed_merges_every_kind_of_event() {
    let f=Fixture::new().await;
    sqlx::query("INSERT INTO members (id,uuid,transient_id,email,name,status,created_at) VALUES ('fm','fu','ft','busy@example.test','Busy','paid',datetime('now'))").execute(&f.state.db).await.unwrap();
    sqlx::query("INSERT INTO members (id,uuid,transient_id,email,status,created_at) VALUES ('other','ou','ot','quiet@example.test','free',datetime('now'))").execute(&f.state.db).await.unwrap();
    let events=&f.state;
    ghost_api::member_events::record_signup(events,"fm","member",&Default::default()).await.unwrap();
    ghost_api::member_events::record_login(events,"fm").await.unwrap();
    ghost_api::member_events::record_status_change(events,"fm",Some("free"),"paid").await.unwrap();
    ghost_api::member_events::record_email_change(events,"fm","old@example.test","busy@example.test").await.unwrap();
    ghost_api::member_events::record_newsletter_change(events,"fm",None,false,"member").await.unwrap();
    ghost_api::member_events::record_payment(events,"fm",1200,Some("usd"),"stripe").await.unwrap();
    ghost_api::member_events::record_subscription_created(events,"fm",Some("sub_1"),&Default::default()).await.unwrap();
    ghost_api::member_events::record_cancellation(events,"fm",Some("sub_1"),Some("price_1")).await.unwrap();
    ghost_api::member_events::record_paid_change(events,ghost_api::member_events::PaidChange{
        member_id:"fm",kind:"created",subscription_id:Some("sub_1"),from_plan:None,to_plan:Some("price_1"),
        from_status:None,to_status:Some("active"),currency:Some("usd"),source:"stripe",mrr_delta:500,
    }).await.unwrap();
    ghost_api::member_events::record_signup(events,"other","admin",&Default::default()).await.unwrap();

    // Emails this member was sent, which only exist because sending now
    // writes a row per recipient.
    sqlx::query("INSERT INTO emails (id,post_id,status,email_count,created_at) VALUES ('fe','fp','submitted',1,datetime('now'))").execute(&f.state.db).await.unwrap();
    ghost_api::newsletter_events::record_recipient(events,ghost_api::newsletter_events::RecipientRecord{
        email_id:"fe",batch_id:None,member_id:"fm",member_uuid:"fu",email:"busy@example.test",provider_id:Some("p1"),error:None,
    }).await.unwrap();

    let (status,feed)=f.request("GET","/admin/members/fm/events/",Value::Null,None).await;
    assert_eq!(status,StatusCode::OK,"{feed}");
    let kinds:Vec<&str>=feed["events"].as_array().unwrap().iter().filter_map(|e|e["type"].as_str()).collect();
    for expected in ["signup_event","login_event","email_change_event","newsletter_event","payment_event","subscription_event","cancel_event","email_sent_event","email_delivered_event"] {
        assert!(kinds.contains(&expected),"{expected} is missing from {kinds:?}");
    }
    // One member's feed is one member's: the other member's signup is not in it.
    assert!(feed["events"].as_array().unwrap().iter().all(|e|e["data"]["member"]["id"]=="fm"),"{feed}");
    // Newest first, which is what makes a feed readable.
    let times:Vec<&str>=feed["events"].as_array().unwrap().iter().filter_map(|e|e["data"]["created_at"].as_str()).collect();
    assert!(times.windows(2).all(|w|w[0]>=w[1]),"{times:?}");
    // The payment says what was paid.
    let payment=feed["events"].as_array().unwrap().iter().find(|e|e["type"]=="payment_event");
    assert!(payment.is_some(),"{feed}");

    // The whole-site feed has both members, and filters to a type on request.
    let (_,all)=f.request("GET","/admin/members/events/",Value::Null,None).await;
    let members:std::collections::HashSet<&str>=all["events"].as_array().unwrap().iter().filter_map(|e|e["data"]["member"]["id"].as_str()).collect();
    assert!(members.contains("fm") && members.contains("other"),"{members:?}");
    let (_,only)=f.request("GET","/admin/members/events/?filter=type:[login_event,payment_event]",Value::Null,None).await;
    assert!(!only["events"].as_array().unwrap().is_empty());
    assert!(only["events"].as_array().unwrap().iter().all(|e|matches!(e["type"].as_str(),Some("login_event"|"payment_event"))),"{only}");

    assert_eq!(f.request("GET","/admin/members/nobody/events/",Value::Null,None).await.0,StatusCode::NOT_FOUND);
    let (_,cookie)=f.staff("Contributor","events-contributor").await;
    let response=f.app.clone().oneshot(Request::builder().uri("/admin/members/events/").header("cookie",&cookie).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(),StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn member_and_revenue_stats_are_built_from_the_event_timeline() {
    let f=Fixture::new().await;
    for (id,status) in [("s1","paid"),("s2","free"),("s3","comped")] {
        sqlx::query("INSERT INTO members (id,uuid,transient_id,email,status,created_at) VALUES (?,?,?,?,?,datetime('now'))")
            .bind(id).bind(format!("u{id}")).bind(format!("t{id}")).bind(format!("{id}@example.test")).bind(status)
            .execute(&f.state.db).await.unwrap();
        ghost_api::member_events::record_signup(&f.state,id,"member",&Default::default()).await.unwrap();
    }
    // An upgrade partway through: the member counts as paid from the day it
    // happened, not from the day they signed up.
    ghost_api::member_events::record_status_change(&f.state,"s1",Some("free"),"paid").await.unwrap();

    let (status,counts)=f.request("GET","/admin/stats/member_count/?days=7",Value::Null,None).await;
    assert_eq!(status,StatusCode::OK,"{counts}");
    assert_eq!(counts["meta"]["totals"]["paid"],1);
    assert_eq!(counts["meta"]["totals"]["free"],1);
    assert_eq!(counts["meta"]["totals"]["comped"],1);
    let today=counts["stats"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(today["free"],3,"the three signups are three free members on the day");
    assert_eq!(today["paid"],1,"the upgrade is a paid member on the day it happened");

    // MRR is a running total of deltas, so a cancellation takes it back down
    // rather than leaving history that never happened.
    for (kind,delta) in [("created",1000),("updated",500),("canceled",-400)] {
        ghost_api::member_events::record_paid_change(&f.state,ghost_api::member_events::PaidChange{
            member_id:"s1",kind,subscription_id:Some("sub_1"),from_plan:None,to_plan:None,
            from_status:None,to_status:None,currency:Some("usd"),source:"stripe",mrr_delta:delta,
        }).await.unwrap();
    }
    let (_,revenue)=f.request("GET","/admin/stats/mrr/?days=7",Value::Null,None).await;
    assert_eq!(revenue["meta"]["totals"][0]["mrr"],1100);
    assert_eq!(revenue["meta"]["totals"][0]["currency"],"usd");
    assert_eq!(revenue["stats"].as_array().unwrap().last().unwrap()["mrr"],1100);

    // Newsletter subscribers: both directions are events, so an unsubscribe
    // is visible rather than just absent.
    sqlx::query("INSERT INTO newsletters (id,name,slug,created_at) VALUES ('nl','Weekly','weekly',datetime('now'))").execute(&f.state.db).await.unwrap();
    ghost_api::member_events::record_newsletter_change(&f.state,"s1",Some("nl"),true,"member").await.unwrap();
    ghost_api::member_events::record_newsletter_change(&f.state,"s2",Some("nl"),true,"member").await.unwrap();
    ghost_api::member_events::record_newsletter_change(&f.state,"s2",Some("nl"),false,"member").await.unwrap();
    let (_,subscribers)=f.request("GET","/admin/stats/subscriber-count/?days=7",Value::Null,None).await;
    let today=subscribers["stats"].as_array().unwrap().last().unwrap().clone();
    assert_eq!((today["subscribed"].as_i64(),today["unsubscribed"].as_i64(),today["net"].as_i64()),(Some(2),Some(1),Some(1)));

    // These are per-staff-member figures about the business, so they are not
    // to be cached by anything between here and the browser.
    let response=f.app.clone().oneshot(Request::builder().uri("/admin/stats/mrr/").header("cookie",&f.cookie).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.headers()["cache-control"],"private, no-store");
}

#[tokio::test]
async fn traffic_stats_report_sources_locations_and_one_posts_numbers() {
    let f=Fixture::new().await;
    let (_,created)=f.request("POST","/admin/posts/",json!({"posts":[{"title":"Read me","slug":"read-me","status":"published"}]}),None).await;
    let post_id=created["posts"][0]["id"].as_str().unwrap().to_string();
    for (path,viewer,source,medium,country) in [
        ("/read-me/","v1",Some("Google"),Some("search"),Some("GB")),
        ("/read-me/","v2",Some("Google"),Some("search"),Some("GB")),
        ("/read-me/","v3",None,None,Some("US")),
        ("/other/","v4",Some("Twitter"),Some("social"),None),
    ] {
        sqlx::query("INSERT INTO page_views (id,path,viewer,audience,created_at,referrer_source,referrer_medium,country) VALUES (?,?,?,'public',datetime('now'),?,?,?)")
            .bind(ghost_core::ids::new_id()).bind(path).bind(viewer).bind(source).bind(medium).bind(country)
            .execute(&f.state.db).await.unwrap();
    }

    let (status,stats)=f.request("GET",&format!("/admin/stats/posts/{post_id}/stats/"),Value::Null,None).await;
    assert_eq!(status,StatusCode::OK,"{stats}");
    assert_eq!(stats["stats"][0]["views"],3);
    assert_eq!(stats["stats"][0]["visitors"],3);
    // Not emailed, so there is no email section rather than a row of zeros.
    assert!(stats["stats"][0]["email"].is_null(),"{stats}");

    let (_,refs)=f.request("GET",&format!("/admin/stats/posts/{post_id}/top-referrers/"),Value::Null,None).await;
    assert_eq!(refs["stats"][0]["source"],"Google");
    assert_eq!(refs["stats"][0]["views"],2);
    // A visit with no referrer is direct traffic, which is an answer.
    assert!(refs["stats"].as_array().unwrap().iter().any(|r|r["source"]=="Direct"),"{refs}");

    let (_,content)=f.request("GET","/admin/stats/top-content/?days=7",Value::Null,None).await;
    assert_eq!(content["stats"][0]["path"],"/read-me/");
    assert_eq!(content["stats"][0]["views"],3);

    let (_,sources)=f.request("GET","/admin/stats/referrers/?days=7",Value::Null,None).await;
    assert_eq!(sources["stats"][0]["source"],"Google");
    assert_eq!(sources["stats"][0]["medium"],"search");

    let (_,places)=f.request("GET","/admin/stats/locations/?days=7",Value::Null,None).await;
    let gb=places["stats"].as_array().unwrap().iter().find(|p|p["country"]=="GB").unwrap().clone();
    assert_eq!(gb["visitors"],2);
    // A view with no country is unknown rather than guessed at.
    assert!(places["stats"].as_array().unwrap().iter().any(|p|p["country"].is_null()),"{places}");

    let (_,visitors)=f.request("POST","/admin/stats/posts-visitor-counts/",json!({"ids":[post_id]}),None).await;
    assert_eq!(visitors["stats"][0][&post_id],3);

    let (_,views)=f.request("GET","/admin/stats/top-posts-views/?days=7",Value::Null,None).await;
    assert_eq!(views["stats"][0]["post_id"],post_id.as_str());
    assert_eq!(views["stats"][0]["views"],3);
}

#[tokio::test]
async fn a_posts_email_figures_come_from_its_recipient_records() {
    let f=Fixture::new().await;
    let (_,created)=f.request("POST","/admin/posts/",json!({"posts":[{"title":"Emailed","slug":"emailed","status":"published"}]}),None).await;
    let post_id=created["posts"][0]["id"].as_str().unwrap().to_string();
    sqlx::query("INSERT INTO emails (id,post_id,status,email_count,delivered_count,opened_count,failed_count,subject,created_at) VALUES ('pe',?,'submitted',10,8,4,2,'Emailed',datetime('now'))")
        .bind(&post_id).execute(&f.state.db).await.unwrap();
    sqlx::query("INSERT INTO members (id,uuid,transient_id,email,status,created_at) VALUES ('cm','cu','ct','clicker@example.test','free',datetime('now'))").execute(&f.state.db).await.unwrap();
    sqlx::query("INSERT INTO newsletter_links (id,post_id,from_url,to_url,created_at,updated_at) VALUES ('nl1',?,'https://a.example','https://a.example',datetime('now'),datetime('now'))")
        .bind(&post_id).execute(&f.state.db).await.unwrap();
    sqlx::query("INSERT INTO newsletter_clicks (id,link_id,member_uuid,created_at) VALUES ('c1','nl1','cu',datetime('now'))").execute(&f.state.db).await.unwrap();

    let (_,stats)=f.request("GET",&format!("/admin/stats/posts/{post_id}/stats/"),Value::Null,None).await;
    let email=stats["stats"][0]["email"].clone();
    assert_eq!(email["sent"],10);
    assert_eq!(email["delivered"],8);
    assert_eq!(email["opened"],4);
    assert_eq!(email["failed"],2);
    assert_eq!(email["clicked"],1);
    // Rates are over what was delivered, not over what was sent.
    assert_eq!(email["open_rate"],50);
    assert_eq!(email["click_rate"],13);

    let (_,newsletters)=f.request("GET","/admin/stats/newsletter-stats/?days=7",Value::Null,None).await;
    assert_eq!(newsletters["stats"][0]["title"],"Emailed");
    assert_eq!(newsletters["stats"][0]["open_rate"],50);

    // A send with nothing delivered yet has an unknown rate, not a zero one.
    sqlx::query("UPDATE emails SET delivered_count=0, opened_count=0 WHERE id='pe'").execute(&f.state.db).await.unwrap();
    let (_,pending)=f.request("GET",&format!("/admin/stats/posts/{post_id}/stats/"),Value::Null,None).await;
    assert!(pending["stats"][0]["email"]["open_rate"].is_null(),"{pending}");
}

#[tokio::test]
async fn a_checkouts_page_history_is_trimmed_to_fit_stripes_metadata() {
    // Stripe caps a metadata value at 500 characters, and a browsing session
    // can be longer than that. The oldest entries go first, because the most
    // recent page is the one a signup is credited to.
    let long: Vec<Value> = (0..60)
        .map(|n| json!({"path": format!("/a-fairly-long-post-slug-number-{n}/"), "referrerSource": "Google", "title": "dropped"}))
        .collect();
    let compact = ghost_api::members::compact_url_history_for_test(Some(&json!(long))).unwrap();
    assert!(compact.len() <= 500, "{} characters", compact.len());
    let kept: Value = serde_json::from_str(&compact).unwrap();
    let kept = kept.as_array().unwrap();
    assert!(!kept.is_empty());
    // The last page browsed survives; the first does not.
    assert_eq!(kept.last().unwrap()["path"], "/a-fairly-long-post-slug-number-59/");
    assert!(!compact.contains("number-0/"), "the oldest entry was kept over a newer one");
    // Only the fields attribution reads are carried.
    assert!(kept[0].get("title").is_none(), "an unused field was sent to Stripe");
    assert!(kept[0].get("referrerSource").is_some());

    // Nothing to carry is not an error, and an entry with nothing usable in
    // it is dropped rather than sent as an empty object.
    assert_eq!(ghost_api::members::compact_url_history_for_test(None), None);
    assert_eq!(ghost_api::members::compact_url_history_for_test(Some(&json!([{"other": "x"}]))), None);
    let one = ghost_api::members::compact_url_history_for_test(Some(&json!([{"path": "/p/"}]))).unwrap();
    assert_eq!(one, r#"[{"path":"/p/"}]"#);
}

#[tokio::test]
async fn a_webmention_is_only_kept_when_the_source_really_links_here() {
    let f=Fixture::new().await;
    let (_,created)=f.request("POST","/admin/posts/",json!({"posts":[{"title":"Linked to","slug":"linked-to","status":"published"}]}),None).await;
    let post_id=created["posts"][0]["id"].as_str().unwrap().to_string();

    let source="https://theirs.example/their-post/";
    let target="https://example.test/linked-to/";

    // A source that does not link here is refused, and nothing is stored.
    let no_link="<html><head><title>Theirs</title></head><body><p>No link.</p></body></html>";
    assert!(ghost_api::mentions::store_verified(&f.state,source,target,no_link).await.is_err());
    let stored:i64=sqlx::query_scalar("SELECT COUNT(*) FROM mentions").fetch_one(&f.state.db).await.unwrap();
    assert_eq!(stored,0,"an unverified mention was stored");

    // Now it does link here, and says who it is.
    let linked=format!(
        r#"<html><head><title>Fallback</title><meta property="og:title" content="Their post">
        <meta property="og:site_name" content="Their Blog">
        <meta name="description" content="What they said."></head>
        <body><p>See <a href="{target}">this</a>.</p></body></html>"#
    );
    ghost_api::mentions::store_verified(&f.state,source,target,&linked).await.unwrap();
    let row:(String,Option<String>,Option<String>,Option<String>,Option<String>,bool,bool)=sqlx::query_as(
        "SELECT source,source_title,source_site_title,source_excerpt,resource_id,verified,deleted FROM mentions",
    ).fetch_one(&f.state.db).await.unwrap();
    assert_eq!(row.0,source);
    assert_eq!(row.1.as_deref(),Some("Their post"));
    assert_eq!(row.2.as_deref(),Some("Their Blog"));
    assert_eq!(row.3.as_deref(),Some("What they said."));
    // The target resolved to the post it points at, which is what lets a
    // mention be shown against a post.
    assert_eq!(row.4.as_deref(),Some(post_id.as_str()));
    assert!(row.5 && !row.6);

    // Sending it twice updates the one mention rather than adding a second.
    ghost_api::mentions::store_verified(&f.state,source,target,&linked).await.unwrap();
    let total:i64=sqlx::query_scalar("SELECT COUNT(*) FROM mentions").fetch_one(&f.state.db).await.unwrap();
    assert_eq!(total,1);

    let (status,mentions)=f.request("GET","/admin/mentions/",Value::Null,None).await;
    assert_eq!(status,StatusCode::OK,"{mentions}");
    assert_eq!(mentions["mentions"][0]["source_site_title"],"Their Blog");
    assert_eq!(mentions["mentions"][0]["resource_title"],"Linked to");

    // A source that has stopped linking here is marked withdrawn, not left
    // standing as a mention that is no longer true.
    assert!(ghost_api::mentions::store_verified(&f.state,source,target,"<html><body><p>Removed.</p></body></html>").await.is_err());
    let deleted:bool=sqlx::query_scalar("SELECT deleted FROM mentions").fetch_one(&f.state.db).await.unwrap();
    assert!(deleted);
    let (_,after)=f.request("GET","/admin/mentions/",Value::Null,None).await;
    assert!(after["mentions"].as_array().unwrap().is_empty(),"{after}");
}

#[tokio::test]
async fn an_incoming_recommendation_is_a_mention_from_another_sites_recommendations_file() {
    let f=Fixture::new().await;
    let now=ghost_core::dates::now_db();
    for (id,source,title) in [
        ("m1","https://friend.example/.well-known/recommendations.json","Friend"),
        ("m2","https://stranger.example/.well-known/recommendations.json","Stranger"),
        ("m3","https://someone.example/a-post/","Not a recommendation"),
    ] {
        sqlx::query("INSERT INTO mentions (id,source,source_site_title,target,verified,deleted,revalidation_failure_count,created_at) VALUES (?,?,?,'https://example.test/',1,0,0,?)")
            .bind(id).bind(source).bind(title).bind(&now).execute(&f.state.db).await.unwrap();
    }
    // This site recommends the first one back.
    sqlx::query("INSERT INTO recommendations (id,url,title,description,one_click_subscribe,featured,created_at,updated_at) VALUES ('r1','https://friend.example','Friend','',0,0,?,?)")
        .bind(&now).bind(&now).execute(&f.state.db).await.unwrap();

    let (status,incoming)=f.request("GET","/admin/incoming_recommendations/",Value::Null,None).await;
    assert_eq!(status,StatusCode::OK,"{incoming}");
    let list=incoming["incoming_recommendations"].as_array().unwrap();
    // The ordinary link is not a recommendation.
    assert_eq!(list.len(),2,"{incoming}");
    let friend=list.iter().find(|r|r["url"]=="https://friend.example").unwrap();
    assert_eq!(friend["title"],"Friend");
    // Mutual, which is the thing the screen exists to tell you.
    assert_eq!(friend["recommending_back"],true);
    let stranger=list.iter().find(|r|r["url"]=="https://stranger.example").unwrap();
    assert_eq!(stranger["recommending_back"],false);

    // And this site publishes what it recommends, so another Ghost can find it.
    let public=ghost_api::mentions::public_router().with_state(f.state.clone());
    let response=public.oneshot(Request::builder().uri("/.well-known/recommendations.json").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(),StatusCode::OK);
    let published:Value=serde_json::from_slice(&to_bytes(response.into_body(),100_000).await.unwrap()).unwrap();
    assert_eq!(published[0]["url"],"https://friend.example");
}

#[tokio::test]
async fn notifications_are_raised_once_and_dismissed_per_user() {
    let f=Fixture::new().await;
    // Raising the same notice twice leaves one: a condition that keeps
    // recurring should not fill the board.
    ghost_api::notifications::notify(&f.state,"stripe-key","alert","Your Stripe key has expired.").await.unwrap();
    ghost_api::notifications::notify(&f.state,"stripe-key","alert","Your Stripe key has expired.").await.unwrap();
    let (status,list)=f.request("GET","/admin/notifications/",Value::Null,None).await;
    assert_eq!(status,StatusCode::OK,"{list}");
    assert_eq!(list["notifications"].as_array().unwrap().len(),1,"{list}");
    let id=list["notifications"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(list["notifications"][0]["message"],"Your Stripe key has expired.");

    // An integration can raise one too.
    let (status,added)=f.request("POST","/admin/notifications/",json!({"notifications":[{"message":"Deploy finished","status":"alert"}]}),None).await;
    assert_eq!(status,StatusCode::CREATED,"{added}");
    assert_eq!(f.request("POST","/admin/notifications/",json!({"notifications":[{"message":"  "}]}),None).await.0,StatusCode::UNPROCESSABLE_ENTITY);

    // Dismissing is per user: the owner stops seeing it, another admin does not.
    assert_eq!(f.request("DELETE",&format!("/admin/notifications/{id}/"),Value::Null,None).await.0,StatusCode::NO_CONTENT);
    let (_,mine)=f.request("GET","/admin/notifications/",Value::Null,None).await;
    assert!(mine["notifications"].as_array().unwrap().iter().all(|n|n["id"]!=id.as_str()),"{mine}");
    let (_,cookie)=f.staff("Administrator","other-admin").await;
    let response=f.app.clone().oneshot(Request::builder().uri("/admin/notifications/").header("cookie",&cookie).body(Body::empty()).unwrap()).await.unwrap();
    let theirs:Value=serde_json::from_slice(&to_bytes(response.into_body(),100_000).await.unwrap()).unwrap();
    assert!(theirs["notifications"].as_array().unwrap().iter().any(|n|n["id"]==id.as_str()),"{theirs}");

    assert_eq!(f.request("DELETE","/admin/notifications/nope/",Value::Null,None).await.0,StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_member_can_sign_in_with_the_code_from_the_email() {
    let f=Fixture::new().await;
    let mut state=f.state.clone();
    state.settings=Arc::new(state.settings.with_overrides([
        ("members_email_otc_enabled".into(),json!(true)),
        ("members_otc_secret".into(),json!("fixture-otc-secret")),
        ("theme_session_secret".into(),json!("fixture-theme-secret")),
    ]));
    let app=ghost_api::members::members_router().with_state(state.clone());

    let integrity=async |app: Router| {
        let response=app.oneshot(Request::builder().uri("/members/api/integrity-token").body(Body::empty()).unwrap()).await.unwrap();
        String::from_utf8(to_bytes(response.into_body(),10_000).await.unwrap().to_vec()).unwrap()
    };
    let token=integrity(app.clone()).await;
    let response=app.clone().oneshot(Request::builder().method("POST").uri("/members/api/send-magic-link")
        .header("content-type","application/json")
        .body(Body::from(json!({"email":"reader@example.test","emailType":"signup","integrityToken":token}).to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(),StatusCode::CREATED);
    let body:Value=serde_json::from_slice(&to_bytes(response.into_body(),10_000).await.unwrap()).unwrap();
    // The reference names the token without being it: holding it is not
    // enough to sign in, which is the point of a code arriving by email.
    let reference=body["otcRef"].as_str().expect("an otcRef was not issued").to_string();
    let raw:String=sqlx::query_scalar("SELECT token FROM tokens WHERE id=?").bind(&reference).fetch_one(&state.db).await.unwrap();
    assert_ne!(reference,raw);

    let code=ghost_api::members::one_time_code_for_test(&state,&raw);
    assert_eq!(code.len(),6);
    assert!(code.chars().all(|c|c.is_ascii_digit()),"{code}");

    let verify=async |app: Router, payload: Value| {
        app.oneshot(Request::builder().method("POST").uri("/members/api/verify-otc")
            .header("content-type","application/json").body(Body::from(payload.to_string())).unwrap()).await.unwrap()
    };
    // A wrong code is refused and costs an attempt.
    let token2=integrity(app.clone()).await;
    let response=verify(app.clone(),json!({"otcRef":reference,"otc":"000000","integrityToken":token2})).await;
    assert_eq!(response.status(),StatusCode::UNAUTHORIZED);
    let attempts:i64=sqlx::query_scalar("SELECT otc_used_count FROM tokens WHERE id=?").bind(&reference).fetch_one(&state.db).await.unwrap();
    assert_eq!(attempts,1);
    // So is a made-up reference, with the same answer, so the endpoint
    // cannot be used to tell a real one from a guess.
    let token3=integrity(app.clone()).await;
    assert_eq!(verify(app.clone(),json!({"otcRef":"nope","otc":code,"integrityToken":token3})).await.status(),StatusCode::UNAUTHORIZED);

    // The right code signs them in and sets the session cookie.
    let token4=integrity(app.clone()).await;
    let response=verify(app.clone(),json!({"otcRef":reference,"otc":code,"integrityToken":token4})).await;
    assert_eq!(response.status(),StatusCode::OK);
    assert!(response.headers().get("set-cookie").is_some(),"no session cookie was set");
    let member:(String,String)=sqlx::query_as("SELECT email,status FROM members WHERE email='reader@example.test'").fetch_one(&state.db).await.unwrap();
    assert_eq!(member,("reader@example.test".into(),"free".into()));
    // Single use, like the link: the token is spent.
    let token5=integrity(app.clone()).await;
    assert_eq!(verify(app,json!({"otcRef":reference,"otc":code,"integrityToken":token5})).await.status(),StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_comment_can_be_disliked_as_well_as_liked() {
    let f=Fixture::new().await;
    let mut state=f.state.clone();
    state.settings=Arc::new(state.settings.with_overrides([("comments_enabled".into(),json!("all"))]));
    let (_,created)=f.request("POST","/admin/posts/",json!({"posts":[{"title":"Discussed","status":"published"}]}),None).await;
    let post_id=created["posts"][0]["id"].as_str().unwrap().to_string();
    sqlx::query("INSERT INTO members (id,uuid,transient_id,email,name,status,created_at) VALUES ('dm','du','dt','reactor@example.test','R','free',datetime('now'))")
        .execute(&state.db).await.unwrap();
    sqlx::query("INSERT INTO comments (id,post_id,member_id,html,status,created_at,updated_at) VALUES ('c1',?,'dm','<p>Hi</p>','published',datetime('now'),datetime('now'))")
        .bind(&post_id).execute(&state.db).await.unwrap();

    let app=ghost_api::comments::comments_router().with_state(state.clone());
    // A real signed member cookie: the signature is what makes it a session.
    let cookie=ghost_api::members::member_cookie_for_test(&state,"dt");
    let react=|app: Router, method: &'static str, path: String, cookie: String| async move {
        app.oneshot(Request::builder().method(method).uri(path).header("cookie",cookie).body(Body::empty()).unwrap()).await.unwrap()
    };
    assert_eq!(react(app.clone(),"POST","/members/api/comments/c1/dislike/".into(),cookie.clone()).await.status(),StatusCode::NO_CONTENT);
    let score:i64=sqlx::query_scalar("SELECT score FROM comment_likes WHERE comment_id='c1'").fetch_one(&state.db).await.unwrap();
    assert_eq!(score,-1,"a dislike is stored as a negative score");
    // Twice is refused, with its own wording.
    assert_eq!(react(app.clone(),"POST","/members/api/comments/c1/dislike/".into(),cookie.clone()).await.status(),StatusCode::BAD_REQUEST);

    // Liking something they had disliked moves the reaction rather than
    // leaving both: a member has one opinion per comment.
    assert_eq!(react(app.clone(),"POST","/members/api/comments/c1/like/".into(),cookie.clone()).await.status(),StatusCode::NO_CONTENT);
    let rows:Vec<i64>=sqlx::query_scalar("SELECT score FROM comment_likes WHERE comment_id='c1'").fetch_all(&state.db).await.unwrap();
    assert_eq!(rows,[1]);

    // The counts are reported separately, and a dislike is not a like.
    let response=react(app.clone(),"GET",format!("/members/api/comments/post/{post_id}/"),cookie.clone()).await;
    let body:Value=serde_json::from_slice(&to_bytes(response.into_body(),1_000_000).await.unwrap()).unwrap();
    assert_eq!(body["comments"][0]["count"]["likes"],1);
    assert_eq!(body["comments"][0]["count"]["dislikes"],0);
    assert_eq!(body["comments"][0]["liked"],true);

    // Removing a reaction that is not there is a 404, per direction.
    assert_eq!(react(app.clone(),"DELETE","/members/api/comments/c1/dislike/".into(),cookie.clone()).await.status(),StatusCode::NOT_FOUND);
    assert_eq!(react(app.clone(),"DELETE","/members/api/comments/c1/like/".into(),cookie.clone()).await.status(),StatusCode::NO_CONTENT);

    // And a moderator can see who reacted which way.
    sqlx::query("INSERT INTO comment_likes (id,comment_id,member_id,score,created_at) VALUES ('l1','c1','dm',-1,datetime('now'))")
        .execute(&f.state.db).await.unwrap();
    let (status,dislikes)=f.request("GET","/admin/comments/c1/dislikes/",Value::Null,None).await;
    assert_eq!(status,StatusCode::OK,"{dislikes}");
    assert_eq!(dislikes["dislikes"][0]["email"],"reactor@example.test");
    let (_,likes)=f.request("GET","/admin/comments/c1/likes/",Value::Null,None).await;
    assert!(likes["likes"].as_array().unwrap().is_empty(),"a dislike was counted as a like: {likes}");
}

#[tokio::test]
async fn an_identity_token_is_signed_and_verifiable_against_the_published_key() {
    let f=Fixture::new().await;
    let (status,identity)=f.request("GET","/admin/identities/",Value::Null,None).await;
    assert_eq!(status,StatusCode::OK,"{identity}");
    let token=identity["identities"][0]["token"].as_str().expect("no token issued").to_string();

    let public=ghost_api::mentions::public_router();
    let _=public; // the jwks route lives on the identity router
    let app=ghost_api::identity::public_router().with_state(f.state.clone());
    let response=app.clone().oneshot(Request::builder().uri("/.well-known/jwks.json").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(),StatusCode::OK);
    let jwks:Value=serde_json::from_slice(&to_bytes(response.into_body(),100_000).await.unwrap()).unwrap();
    let kid=jwks["keys"][0]["kid"].as_str().unwrap();
    // The key in the set is the key that signed, which is how a verifier
    // picks the right one.
    assert_eq!(jsonwebtoken::decode_header(&token).unwrap().kid.as_deref(),Some(kid));
    assert_eq!(jwks["keys"][0]["alg"],"RS256");

    let (_,public_pem)=ghost_api::identity::keys(&f.state).await.unwrap();
    let claims=ghost_api::identity::verify(&public_pem,&token,"https://example.test").expect("the token did not verify");
    assert_eq!(claims["sub"],"owner@example.test");

    // Unauthenticated callers get no token, and a member with no session
    // gets no entitlement token either.
    let response=f.app.clone().oneshot(Request::builder().uri("/admin/identities/").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(),StatusCode::UNAUTHORIZED);
    let response=app.oneshot(Request::builder().uri("/members/api/entitlements").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(),StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_rest_surface_ghosts_clients_expect_is_reachable() {
    let f=Fixture::new().await;
    let (_,created)=f.request("POST","/admin/posts/",json!({"posts":[{"title":"A Post About Things","status":"published"}]}),None).await;
    let post_id=created["posts"][0]["id"].as_str().unwrap().to_string();

    // Slug generation, which the editor asks for before saving.
    let (status,slug)=f.request("GET","/admin/slugs/post/A%20Post%20About%20Things/",Value::Null,None).await;
    assert_eq!(status,StatusCode::OK,"{slug}");
    // The title is taken, so the next one is suffixed.
    assert_eq!(slug["slugs"][0]["slug"],"a-post-about-things-2");
    // Excluding the post itself gives it its own slug back.
    let (_,own)=f.request("GET",&format!("/admin/slugs/post/A%20Post%20About%20Things/{post_id}/"),Value::Null,None).await;
    assert_eq!(own["slugs"][0]["slug"],"a-post-about-things");
    assert_eq!(f.request("GET","/admin/slugs/nonsense/x/",Value::Null,None).await.0,StatusCode::UNPROCESSABLE_ENTITY);

    // What the admin client needs to know about the install, and nothing secret.
    let (_,config)=f.request("GET","/admin/config/",Value::Null,None).await;
    assert_eq!(config["config"]["database"],"sqlite3");
    assert_eq!(config["config"]["version"],ghost_core::GHOST_VERSION);
    let serialised=config.to_string();
    assert!(!serialised.contains("secret") && !serialised.contains("sk_"),"{serialised}");

    // Staff lookups by email and slug, which Ghost's clients use.
    let (status,user)=f.request("GET","/admin/users/email/owner@example.test/",Value::Null,None).await;
    assert_eq!(status,StatusCode::OK,"{user}");
    assert_eq!(user["users"][0]["email"],"owner@example.test");
    let slug=user["users"][0]["slug"].as_str().unwrap().to_string();
    let (_,by_slug)=f.request("GET",&format!("/admin/users/slug/{slug}/"),Value::Null,None).await;
    assert_eq!(by_slug["users"][0]["email"],"owner@example.test");
    assert_eq!(f.request("GET","/admin/users/email/nobody@example.test/",Value::Null,None).await.0,StatusCode::NOT_FOUND);

    // A staff token is your own and nobody else's.
    let owner=user["users"][0]["id"].as_str().unwrap().to_string();
    let (status,minted)=f.request("PUT",&format!("/admin/users/{owner}/token/"),Value::Null,None).await;
    assert_eq!(status,StatusCode::OK,"{minted}");
    let secret=minted["apiKey"]["secret"].as_str().unwrap().to_string();
    let (_,read)=f.request("GET",&format!("/admin/users/{owner}/token/"),Value::Null,None).await;
    assert_eq!(read["apiKey"]["secret"],secret);
    let (_,cookie)=f.staff("Administrator","nosy-admin").await;
    let response=f.app.clone().oneshot(Request::builder().uri(format!("/admin/users/{owner}/token/")).header("cookie",&cookie).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(),StatusCode::FORBIDDEN,"another admin could read a staff token");

    // Recommendations CRUD.
    let (status,added)=f.request("POST","/admin/recommendations/",json!({"recommendations":[{"url":"https://friend.example","title":"Friend"}]}),None).await;
    assert_eq!(status,StatusCode::CREATED,"{added}");
    let rec_id=added["recommendations"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(f.request("POST","/admin/recommendations/",json!({"recommendations":[{"url":"https://friend.example"}]}),None).await.0,StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(f.request("POST","/admin/recommendations/",json!({"recommendations":[{"url":"javascript:alert(1)"}]}),None).await.0,StatusCode::UNPROCESSABLE_ENTITY);
    let (_,edited)=f.request("PUT",&format!("/admin/recommendations/{rec_id}/"),json!({"recommendations":[{"title":"Renamed"}]}),None).await;
    assert_eq!(edited["recommendations"][0]["title"],"Renamed");
    assert_eq!(f.request("DELETE",&format!("/admin/recommendations/{rec_id}/"),Value::Null,None).await.0,StatusCode::NO_CONTENT);
    assert_eq!(f.request("GET",&format!("/admin/recommendations/{rec_id}/"),Value::Null,None).await.0,StatusCode::NOT_FOUND);

    // Search index for pages and staff, which the admin's own search needs.
    let (_,pages)=f.request("GET","/admin/search-index/pages/",Value::Null,None).await;
    assert!(pages["pages"].is_array());
    let (_,staff)=f.request("GET","/admin/search-index/users/",Value::Null,None).await;
    assert!(staff["users"].as_array().unwrap().iter().any(|u|u["slug"]==slug.as_str()));

    // Feedback counts for a post.
    let (_,feedback)=f.request("GET",&format!("/admin/feedback/{post_id}/"),Value::Null,None).await;
    assert_eq!(feedback["feedback"][0]["more_like_this"],0);

    // Signing a member out everywhere moves the id their cookie carries.
    sqlx::query("INSERT INTO members (id,uuid,transient_id,email,status,created_at) VALUES ('sm','su','before','out@example.test','free',datetime('now'))")
        .execute(&f.state.db).await.unwrap();
    assert_eq!(f.request("DELETE","/admin/members/sm/sessions/",Value::Null,None).await.0,StatusCode::NO_CONTENT);
    let transient:String=sqlx::query_scalar("SELECT transient_id FROM members WHERE id='sm'").fetch_one(&f.state.db).await.unwrap();
    assert_ne!(transient,"before");
    assert_eq!(f.request("DELETE","/admin/members/nope/sessions/",Value::Null,None).await.0,StatusCode::NOT_FOUND);

    // Ghost's own spelling of a gift link and of a bulk edit.
    let (status,gift)=f.request("POST",&format!("/admin/posts/{post_id}/gift_links/"),Value::Null,None).await;
    assert_eq!(status,StatusCode::OK,"{gift}");
    // Ghost's body shape, not this port's, since the path is Ghost's.
    let (status,bulk)=f.request("PUT","/admin/posts/bulk/",json!({"bulk":{"action":"draft","meta":{"ids":[post_id]}}}),None).await;
    assert!(status.is_success(),"{bulk}");
    let status_now:String=sqlx::query_scalar("SELECT status FROM posts WHERE id=?").bind(&post_id).fetch_one(&f.state.db).await.unwrap();
    assert_eq!(status_now,"draft","the bulk action did not take effect");
    // And its spelling of an email preview.
    let (status,preview)=f.request("GET",&format!("/admin/email_previews/posts/{post_id}/"),Value::Null,None).await;
    assert_eq!(status,StatusCode::OK,"{preview}");
}

#[tokio::test]
async fn ghost_bulk_delete_requires_selection_and_respects_resource_kind(){
    let f=Fixture::new().await;
    let (_,post)=f.request("POST","/admin/posts/",json!({"posts":[{"title":"Delete me"}]}),None).await;
    let id=post["posts"][0]["id"].as_str().unwrap();
    let (_,kept)=f.request("POST","/admin/pages/",json!({"pages":[{"title":"Keep page"}]}),None).await;
    let page=kept["pages"][0]["id"].as_str().unwrap();
    assert_eq!(f.request("DELETE","/admin/posts/",Value::Null,None).await.0,StatusCode::UNPROCESSABLE_ENTITY);
    let (status,result)=f.request("DELETE",&format!("/admin/posts/?filter=id:{id}"),Value::Null,None).await;
    assert_eq!(status,StatusCode::OK,"{result}");assert_eq!(result["bulk"]["meta"]["stats"]["successful"],1);
    assert_eq!(f.request("GET",&format!("/admin/pages/{page}/"),Value::Null,None).await.0,StatusCode::OK);
}

#[tokio::test]
async fn ghost_bulk_link_payload_is_filtered_and_invalid_filters_never_edit_every_link(){
    let f=Fixture::new().await;
    f.state.settings.set(&f.state.db,"email_track_clicks",&json!(true)).await.unwrap();
    let mut post_ids=Vec::new();
    for title in ["One","Two"] {let (_,post)=f.request("POST","/admin/posts/",json!({"posts":[{"title":title}]}),None).await;post_ids.push(post["posts"][0]["id"].as_str().unwrap().to_string());}
    for id in &post_ids{ghost_api::newsletter_events::track_links(&f.state,id,"<a href=\"https://old.example/path\">Old</a>").await.unwrap();}
    let body=json!({"bulk":{"action":"updateLink","meta":{"link":{"to":"https://new.example/path"}}}});
    assert_eq!(f.request("PUT","/admin/links/bulk/",body.clone(),None).await.0,StatusCode::UNPROCESSABLE_ENTITY);
    let path=format!("/admin/links/bulk/?filter=post_id:{}",post_ids[0]);
    let (status,result)=f.request("PUT",&path,body,None).await;assert_eq!(status,StatusCode::OK,"{result}");assert_eq!(result["bulk"]["meta"]["stats"]["successful"],1);
    let urls:Vec<String>=sqlx::query_scalar("SELECT to_url FROM newsletter_links ORDER BY rowid").fetch_all(&f.state.db).await.unwrap();assert_eq!(urls,["https://new.example/path","https://old.example/path"]);
}

#[tokio::test]
async fn automation_opens_and_clicks_update_the_original_revision_once(){
    use ghost_core::automations::{Action,ActionKind,Status};
    let f=Fixture::new().await;
    f.state.settings.set(&f.state.db,"email_track_opens",&json!(true)).await.unwrap();f.state.settings.set(&f.state.db,"email_track_clicks",&json!(true)).await.unwrap();
    let actions=[Action{id:"tracked-action".into(),kind:ActionKind::SendEmail{subject:"Hello".into(),lexical:"{}".into()}}];
    repos::automations::create(&f.state.db,&repos::automations::AutomationInput{name:"Tracked",description:"",status:Status::Inactive,trigger_tier_scope:None,trigger_tier_ids:&[],actions:&actions,edges:&[]}).await.unwrap();
    let revision:String=sqlx::query_scalar("SELECT id FROM automation_action_revisions WHERE action_id='tracked-action'").fetch_one(&f.state.db).await.unwrap();
    let body=ghost_api::automation_tracking::instrument(&f.state,&revision,"reader-uuid","<body><a href=\"https://example.org/article\">Read</a></body>").await.unwrap();
    assert!(body.contains("/r/a/o/"));assert!(body.contains("/r/a/c/"));
    let token:String=sqlx::query_scalar("SELECT token FROM automation_tracking").fetch_one(&f.state.db).await.unwrap();
    let link:String=sqlx::query_scalar("SELECT id FROM automation_links").fetch_one(&f.state.db).await.unwrap();
    assert!(ghost_api::automation_tracking::record(&f.state,&token,None).await.unwrap());assert!(!ghost_api::automation_tracking::record(&f.state,&token,None).await.unwrap());
    assert!(ghost_api::automation_tracking::record(&f.state,&token,Some(&link)).await.unwrap());assert!(!ghost_api::automation_tracking::record(&f.state,&token,Some(&link)).await.unwrap());
    assert!(!ghost_api::automation_tracking::record(&f.state,&token,Some("other-link")).await.unwrap());
    let counts:(i64,i64)=sqlx::query_as("SELECT email_opened_count,email_clicked_count FROM automation_action_revisions WHERE id=?").bind(revision).fetch_one(&f.state.db).await.unwrap();assert_eq!(counts,(1,1));
}

#[tokio::test]
async fn verification_tokens_are_expiring_and_single_use(){
    use sha2::{Digest,Sha256};
    let f=Fixture::new().await;
    sqlx::query("INSERT INTO newsletters(id,name,slug,created_at)VALUES('verify-nl','Verify','verify',datetime('now'))").execute(&f.state.db).await.unwrap();
    for (token,expiry)in [("valid-verification","+1 day"),("expired-verification","-1 day")]{
        sqlx::query("INSERT INTO email_verifications(token_hash,resource_type,resource_id,field,value,expires_at)VALUES(?,'newsletter','verify-nl','sender_email','verified@example.test',datetime('now',?))").bind(hex::encode(Sha256::digest(token.as_bytes()))).bind(expiry).execute(&f.state.db).await.unwrap();
    }
    assert_eq!(f.request("PUT","/admin/newsletters/verifications/",json!({"token":"expired-verification"}),None).await.0,StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(f.request("PUT","/admin/newsletters/verifications/",json!({"token":"valid-verification"}),None).await.0,StatusCode::OK);
    assert_eq!(f.request("PUT","/admin/newsletters/verifications/",json!({"token":"valid-verification"}),None).await.0,StatusCode::UNPROCESSABLE_ENTITY);
    let email:String=sqlx::query_scalar("SELECT sender_email FROM newsletters WHERE id='verify-nl'").fetch_one(&f.state.db).await.unwrap();assert_eq!(email,"verified@example.test");
}

#[tokio::test]
async fn experimental_app_lifecycle_is_gated_and_cannot_fetch_private_hosts(){
    let f=Fixture::new().await;
    assert_eq!(f.request("GET","/admin/apps/installations/",Value::Null,None).await.0,StatusCode::NOT_FOUND);
    f.state.settings.set(&f.state.db,"labs",&json!({"apps":true})).await.unwrap();
    let (status,body)=f.request("GET","/admin/apps/installations/",Value::Null,None).await;assert_eq!(status,StatusCode::OK);assert_eq!(body["app_installations"],json!([]));
    assert_eq!(f.request("POST","/admin/apps/installations/preview/",json!({"app_installation_previews":[{"manifest_url":"https://127.0.0.1/private.json"}]}),None).await.0,StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn member_csv_api_round_trip_keeps_labels_preferences_and_complimentary_tiers(){
    let f=Fixture::new().await;
    sqlx::query("INSERT INTO products(id,name,slug,type,created_at)VALUES('csv-tier','Gold CSV','gold-csv','paid',datetime('now'))").execute(&f.state.db).await.unwrap();
    sqlx::query("INSERT INTO newsletters(id,name,slug,status,created_at)VALUES('csv-nl','CSV newsletter','csv-nl','active',datetime('now'))").execute(&f.state.db).await.unwrap();
    let csv="email,name,note,subscribed_to_emails,complimentary_plan,labels,tiers\r\nreader-csv@example.test,Reader,Imported,true,true,VIP,Gold CSV\r\n";
    let body=format!("--csv-boundary\r\nContent-Disposition: form-data; name=\"membersfile\"; filename=\"members.csv\"\r\nContent-Type: text/csv\r\n\r\n{csv}\r\n--csv-boundary--\r\n");
    let response=f.app.clone().oneshot(Request::builder().method("POST").uri("/admin/members/upload/").header("cookie",&f.cookie).header("content-type","multipart/form-data; boundary=csv-boundary").body(Body::from(body)).unwrap()).await.unwrap();
    let status=response.status();let bytes=to_bytes(response.into_body(),1_000_000).await.unwrap();assert_eq!(status,StatusCode::OK,"{}",String::from_utf8_lossy(&bytes));
    let response=f.app.clone().oneshot(Request::builder().uri("/admin/members/upload/?filter=email:'reader-csv@example.test'").header("cookie",&f.cookie).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(),StatusCode::OK);let bytes=to_bytes(response.into_body(),1_000_000).await.unwrap();
    let mut reader=csv::Reader::from_reader(bytes.as_ref());let rows:Vec<HashMap<String,String>>=reader.deserialize().collect::<Result<_,_>>().unwrap();assert_eq!(rows.len(),1);assert_eq!(rows[0]["labels"],"VIP");assert_eq!(rows[0]["tiers"],"Gold CSV");assert_eq!(rows[0]["subscribed_to_emails"],"true");assert_eq!(rows[0]["complimentary_plan"],"true");
}

#[tokio::test]
async fn api_key_rotation_revokes_the_previous_content_secret(){
    let f=Fixture::new().await;let integration=f.integration().await;let id=integration["id"].as_str().unwrap();
    let key=&integration["api_keys"].as_array().unwrap().iter().find(|key|key["type"]=="content").unwrap();let old=key["secret"].as_str().unwrap();let key_id=key["id"].as_str().unwrap();
    let (status,result)=f.request("POST",&format!("/admin/integrations/{id}/api_key/{key_id}/refresh/"),json!({}),None).await;assert_eq!(status,StatusCode::OK,"{result}");
    assert_ne!(result["api_keys"][0]["secret"],old);
    assert_eq!(f.request("GET",&format!("/content/posts/?key={old}"),Value::Null,None).await.0,StatusCode::UNAUTHORIZED);
    assert_eq!(f.request("GET",&format!("/content/posts/?key={}",result["api_keys"][0]["secret"].as_str().unwrap()),Value::Null,None).await.0,StatusCode::OK);
}

#[tokio::test]
async fn post_csv_import_preserves_flags_dates_tags_and_renders_source(){
    let f=Fixture::new().await;f.state.settings.set(&f.state.db,"labs",&json!({"csvContentImporter":true})).await.unwrap();
    let csv="title,markdown,published_at,status,featured,tags\r\nCSV post,**Imported body**,2026-10-01T12:34:56Z,published,true,Imported CSV\r\n";
    let body=format!("--posts-boundary\r\nContent-Disposition: form-data; name=\"postsfile\"; filename=\"posts.csv\"\r\nContent-Type: text/csv\r\n\r\n{csv}\r\n--posts-boundary--\r\n");
    let response=f.app.clone().oneshot(Request::builder().method("POST").uri("/admin/posts/upload/").header("cookie",&f.cookie).header("content-type","multipart/form-data; boundary=posts-boundary").body(Body::from(body)).unwrap()).await.unwrap();
    let status=response.status();let body=to_bytes(response.into_body(),1_000_000).await.unwrap();assert_eq!(status,StatusCode::OK,"{}",String::from_utf8_lossy(&body));
    let (html,date,featured):(String,String,bool)=sqlx::query_as("SELECT html,published_at,featured FROM posts WHERE title='CSV post'").fetch_one(&f.state.db).await.unwrap();
    assert!(html.contains("<strong>Imported body</strong>"));assert_eq!(date,"2026-10-01 12:34:56");assert!(featured);
    let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM posts_tags pt JOIN posts p ON p.id=pt.post_id JOIN tags t ON t.id=pt.tag_id WHERE p.title='CSV post' AND t.name='Imported CSV'").fetch_one(&f.state.db).await.unwrap();assert_eq!(count,1);
}

#[tokio::test]
async fn ghost_password_put_accepts_the_standard_password_envelope(){
    let f=Fixture::new().await;let owner=repos::users::find_owner(&f.state.db).await.unwrap().unwrap();
    let hash=bcrypt::hash("old-password-valid",4).unwrap();sqlx::query("UPDATE users SET password=? WHERE id=?").bind(hash).bind(&owner.id).execute(&f.state.db).await.unwrap();
    let (status,body)=f.request("PUT","/admin/users/password/",json!({"password":[{"user_id":owner.id,"oldPassword":"old-password-valid","newPassword":"new-password-valid","ne2Password":"new-password-valid"}]}),None).await;
    assert_eq!(status,StatusCode::NO_CONTENT,"{body}");
    let hash:String=sqlx::query_scalar("SELECT password FROM users WHERE id=?").bind(&owner.id).fetch_one(&f.state.db).await.unwrap();assert!(bcrypt::verify("new-password-valid",&hash).unwrap());
}

/// Host limits: a plan ceiling has to refuse the request, say what the
/// ceiling was, and point at somewhere to do something about it. A
/// self-hosted install configures none of this and is checked alongside, so
/// that a bug in the limit plumbing cannot quietly start refusing requests
/// on the installs that are most of them.
#[tokio::test]
async fn host_limits_refuse_a_create_and_say_what_the_limit_was() {
    let f = Fixture::with_host_settings(json!({
        "billing": {"enabled": true, "url": "https://billing.example/upgrade"},
        "limits": {
            // The seed creates one newsletter, so a ceiling of one is already
            // reached and the next create is the one over the line.
            "newsletters": {"max": 1, "error": "Your plan allows {{max}} newsletter."},
            "staff": {"max": 1},
            "limitSocialWeb": {"disabled": true},
        },
    }))
    .await;

    let (status, body) = f
        .request("POST", "/admin/newsletters/", json!({"newsletters": [{"name": "Second"}]}), None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["errors"][0]["errorType"], "HostLimitError", "{body}");
    assert_eq!(body["errors"][0]["message"], "Your plan allows 1 newsletter.", "{body}");
    assert_eq!(body["errors"][0]["errorDetails"]["limit"], 1, "{body}");
    assert_eq!(body["errors"][0]["errorDetails"]["total"], 1, "{body}");
    assert_eq!(body["errors"][0]["help"], "https://billing.example/upgrade", "{body}");
    // And it did not write the row before refusing.
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM newsletters WHERE name='Second'")
        .fetch_one(&f.state.db)
        .await
        .unwrap();
    assert_eq!(count, 0);

    // The owner already fills a staff ceiling of one, so an invite is over it
    // — and a pending invite counts, which is why this is refused here rather
    // than when somebody tries to accept.
    let (status, body) = f
        .request("POST", "/admin/invites/", json!({"invites": [{"email": "new@example.test", "roles": [{"name": "Author"}]}]}), None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        body["errors"][0]["message"], "This action would exceed the staff limit on your current plan.",
        "{body}"
    );
    // A Contributor is exempt from the staff limit, as in Ghost — so this one
    // gets as far as trying to send the email, rather than being refused.
    let (status, body) = f
        .request("POST", "/admin/invites/", json!({"invites": [{"email": "writer@example.test", "roles": [{"name": "Contributor"}]}]}), None)
        .await;
    assert_ne!(status, StatusCode::FORBIDDEN, "{body}");

    // A flag limit withholds a feature outright, whatever the site's own
    // setting says.
    f.state.settings.set(&f.state.db, "social_web_enabled", &json!(true)).await.unwrap();
    assert!(!ghost_api::federation::enabled(&f.state), "the plan does not include the social web");

    // `hostSettings` reaches the admin client, so its screens can say so.
    let (status, body) = f.request("GET", "/admin/config/", json!({}), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["config"]["hostSettings"]["billing"]["url"], "https://billing.example/upgrade", "{body}");
    assert_eq!(body["config"]["hostSettings"]["limits"]["limitSocialWeb"]["disabled"], true, "{body}");
    // Analytics is this port's own, not a column store's.
    assert_eq!(body["config"]["stats"]["provider"], "native", "{body}");
    // And the shipped labs flags read true whatever is stored.
    assert_eq!(body["config"]["labs"]["editorReact"], true, "{body}");
}

#[tokio::test]
async fn a_self_hosted_install_has_no_limits_to_hit() {
    let f = Fixture::new().await;
    let (status, body) = f
        .request("POST", "/admin/newsletters/", json!({"newsletters": [{"name": "Second"}]}), None)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    f.state.settings.set(&f.state.db, "social_web_enabled", &json!(true)).await.unwrap();
    assert!(ghost_api::federation::enabled(&f.state));

    let (_, body) = f.request("GET", "/admin/config/", json!({}), None).await;
    assert!(body["config"]["hostSettings"].is_null(), "{body}");
}

/// The announcement bar is decided per visitor, which is why it is an
/// endpoint and not part of the rendered page. Each audience in
/// `announcement_visibility` is a separate question, and `visitors` means a
/// signed-out reader specifically rather than "anyone".
#[tokio::test]
async fn an_announcement_is_served_only_to_the_audience_it_was_set_for() {
    let f = Fixture::new().await;
    let state = f.state.clone();
    sqlx::query("INSERT INTO members (id,uuid,transient_id,email,name,status,created_at) VALUES ('fm','fu','ft','free@example.test','F','free',datetime('now'))")
        .execute(&state.db).await.unwrap();
    sqlx::query("INSERT INTO members (id,uuid,transient_id,email,name,status,created_at) VALUES ('pm','pu','pt','paid@example.test','P','comped',datetime('now'))")
        .execute(&state.db).await.unwrap();

    // Markdown in, HTML out: this port's Settings → Site field takes
    // Markdown, and the widget writes HTML into the bar.
    let ask = |state: AppState, cookie: Option<&'static str>| async move {
        let app = ghost_api::members::members_router().with_state(state.clone());
        let mut req = Request::builder().method("GET").uri("/members/api/announcement/");
        if let Some(transient) = cookie {
            req = req.header("cookie", ghost_api::members::member_cookie_for_test(&state, transient));
        }
        let res = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
        let status = res.status();
        let body = to_bytes(res.into_body(), 100_000).await.unwrap();
        (status, String::from_utf8_lossy(&body).to_string())
    };

    // Nothing set: nothing served, to anyone.
    assert_eq!(ask(state.clone(), None).await.0, StatusCode::NO_CONTENT);

    let with = |visibility: Value| {
        Arc::new(state.settings.with_overrides([
            ("announcement_content".into(), json!("We **moved**")),
            ("announcement_background".into(), json!("accent")),
            ("announcement_visibility".into(), visibility),
        ]))
    };

    // Visitors only: the signed-out reader sees it and neither member does.
    let mut visitors = state.clone();
    visitors.settings = with(json!(["visitors"]));
    let (status, body) = ask(visitors.clone(), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("<strong>moved</strong>"), "Markdown is rendered for the widget: {body}");
    assert!(body.contains(r#""announcement_background":"accent""#), "{body}");
    assert_eq!(ask(visitors.clone(), Some("ft")).await.0, StatusCode::NO_CONTENT, "a free member is not a visitor");
    assert_eq!(ask(visitors.clone(), Some("pt")).await.0, StatusCode::NO_CONTENT, "a paid member is not a visitor");

    // Paid only — the case the old server-rendered bar could never serve.
    let mut paid = state.clone();
    paid.settings = with(json!(["paid_members"]));
    assert_eq!(ask(paid.clone(), Some("pt")).await.0, StatusCode::OK);
    assert_eq!(ask(paid.clone(), Some("ft")).await.0, StatusCode::NO_CONTENT);
    assert_eq!(ask(paid.clone(), None).await.0, StatusCode::NO_CONTENT);

    // Free only.
    let mut free = state.clone();
    free.settings = with(json!(["free_members"]));
    assert_eq!(ask(free.clone(), Some("ft")).await.0, StatusCode::OK);
    assert_eq!(ask(free.clone(), Some("pt")).await.0, StatusCode::NO_CONTENT);

    // An empty audience list means nobody, however much content there is.
    let mut nobody = state.clone();
    nobody.settings = with(json!([]));
    assert_eq!(ask(nobody.clone(), None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(ask(nobody, Some("ft")).await.0, StatusCode::NO_CONTENT);
}
