use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use ghost_api::AppState;
use ghost_db::repos;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{atomic::AtomicBool, Arc},
};
use tokio::sync::RwLock;
use tower::ServiceExt;

struct Fixture {
    state: AppState,
    app: Router,
}

impl Fixture {
    async fn new() -> Self {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("../../migrations").run(&db).await.unwrap();
        ghost_db::seed::seed_if_empty(&db).await.unwrap();
        let _owner =
            repos::users::setup_owner(&db, "Test owner", "owner@example.test", "unused-test-hash")
                .await
                .unwrap();
        let content_key = repos::integrations::internal_content_key(&db)
            .await
            .unwrap()
            .unwrap();
        let settings = Arc::new(ghost_db::SettingsCache::from_values([
            ("site_url".into(), json!("https://example.test")),
            ("members_signup_access".into(), json!("all")),
            ("llms_enabled".into(), json!(true)),
            ("comments_enabled".into(), json!("all")),
            ("theme_session_secret".into(), json!("isolated-test-secret")),
        ]));
        let config = serde_json::from_value(json!({
            "url": "https://example.test", "server": {},
            "database": {"connection": {"filename": ":memory:"}}, "paths": {}
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
            site_origin: "https://example.test".into(),
            content_api_key: content_key.clone(),
        };
        let theme = ghost_render::load_and_activate(&theme_dir, &deps, HashMap::new()).unwrap();
        let state = AppState {
            db,
            config: Arc::new(config),
            settings,
            theme: Arc::new(RwLock::new(Arc::new(theme))),
            router: Arc::new(RwLock::new(router)),
            redirects: Arc::new(RwLock::new(Arc::new(
                ghost_routing::redirects::Redirects::default(),
            ))),
            setup_done: Arc::new(AtomicBool::new(true)),
            frontend_key: Arc::new(RwLock::new(Some(content_key.clone()))),
        };
        let app = ghost_front::site_router(state.clone())
            .merge(ghost_api::members::members_router().with_state(state.clone()))
            .merge(ghost_api::comments::comments_router().with_state(state.clone()));
        Self { state, app }
    }

    async fn request(
        &self,
        method: &str,
        path: &str,
        cookie: Option<&str>,
        body: Value,
    ) -> (StatusCode, axum::http::HeaderMap, String) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");
        if let Some(cookie) = cookie {
            request = request.header("cookie", cookie);
        }
        let response = self
            .app
            .clone()
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = to_bytes(response.into_body(), 10_000_000).await.unwrap();
        (status, headers, String::from_utf8_lossy(&body).to_string())
    }

    async fn member(&self, status: &str, tiers: &[String]) -> (String, String) {
        let id = ghost_core::ids::new_id();
        let email = format!("{id}@example.test");
        sqlx::query("INSERT INTO members (id, uuid, transient_id, email, name, status, created_at, updated_at) VALUES (?, ?, ?, ?, 'Test member', ?, datetime('now'), datetime('now'))")
            .bind(&id).bind(ghost_core::ids::uuid_v4()).bind(ghost_core::ids::random_hex(16)).bind(&email).bind(status).execute(&self.state.db).await.unwrap();
        for tier in tiers {
            sqlx::query("INSERT INTO members_products (id, member_id, product_id, sort_order) VALUES (?, ?, ?, 0)")
                .bind(ghost_core::ids::new_id()).bind(&id).bind(tier).execute(&self.state.db).await.unwrap();
        }
        // Exchange a single-use fixture token through the real sign-in handler.
        let token = ghost_core::ids::random_hex(32);
        sqlx::query("INSERT INTO tokens (id, token, uuid, data, created_at) VALUES (?, ?, ?, ?, datetime('now'))")
            .bind(ghost_core::ids::new_id()).bind(&token).bind(ghost_core::ids::uuid_v4()).bind(json!({"email":email,"action":"signin"}).to_string()).execute(&self.state.db).await.unwrap();
        let (status, headers, _) = self
            .request(
                "GET",
                &format!("/members/?token={token}&action=signin"),
                None,
                Value::Null,
            )
            .await;
        assert!(status.is_redirection());
        let cookie = headers
            .get("set-cookie")
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        (id, cookie)
    }

    async fn post(
        &self,
        title: &str,
        visibility: &str,
        tiers: Vec<String>,
        html: &str,
    ) -> ghost_core::models::Post {
        let owner = repos::users::find_owner(&self.state.db)
            .await
            .unwrap()
            .unwrap();
        repos::posts::insert(
            &self.state.db,
            &repos::posts::NewPost {
                title: title.into(),
                visibility: visibility.into(),
                tier_ids: tiers,
                status: "published".into(),
                html: Some(html.into()),
                plaintext: Some(ghost_render::markdown::html_to_plaintext(html)),
                ..Default::default()
            },
            &owner.id,
        )
        .await
        .unwrap()
    }
}

#[tokio::test]
async fn tier_content_and_comments_require_a_current_matching_entitlement() {
    let f = Fixture::new().await;
    let tier: String = sqlx::query_scalar("SELECT id FROM products WHERE type = 'paid' LIMIT 1")
        .fetch_one(&f.state.db)
        .await
        .unwrap();
    let other_tier = ghost_core::ids::new_id();
    sqlx::query("INSERT INTO products (id, name, slug, type, created_at) VALUES (?, 'Other tier', 'other-tier', 'paid', datetime('now'))")
        .bind(&other_tier).execute(&f.state.db).await.unwrap();
    let post = f
        .post(
            "Tier access regression",
            "tiers",
            vec![tier.clone()],
            "<p>Public introduction</p><!--members-only--><p>PRIVATE-TIER-CONTENT</p>",
        )
        .await;
    let path = format!("/{}/", post.slug.as_deref().unwrap());
    let (_, free) = f.member("free", &[]).await;
    let (_, wrong) = f.member("paid", &[other_tier]).await;
    let (member_id, correct) = f.member("paid", &[tier]).await;
    for cookie in [None, Some(free.as_str()), Some(wrong.as_str())] {
        let (status, _, body) = f.request("GET", &path, cookie, Value::Null).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            !body.contains("PRIVATE-TIER-CONTENT"),
            "protected body leaked to {cookie:?}"
        );
    }
    let (status, _, body) = f.request("GET", &path, Some(&correct), Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("PRIVATE-TIER-CONTENT"),
        "matching tier did not receive content"
    );
    let comment = json!({"comments":[{"post_id":post.id,"html":"<p>A tier discussion</p>"}]});
    assert_eq!(
        f.request(
            "POST",
            "/members/api/comments/",
            Some(&wrong),
            comment.clone()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, _, result) = f
        .request(
            "POST",
            "/members/api/comments/",
            Some(&correct),
            comment.clone(),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{result}");
    sqlx::query(
        "UPDATE members_products SET expiry_at = '2000-01-01 00:00:00' WHERE member_id = ?",
    )
    .bind(member_id)
    .execute(&f.state.db)
    .await
    .unwrap();
    assert!(!f
        .request("GET", &path, Some(&correct), Value::Null)
        .await
        .2
        .contains("PRIVATE-TIER-CONTENT"));
    assert_eq!(
        f.request("POST", "/members/api/comments/", Some(&correct), comment)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn markdown_and_public_indexes_never_export_protected_bodies() {
    let f = Fixture::new().await;
    let post = f
        .post(
            "Markdown membership regression",
            "members",
            vec![],
            "<p>Safe preview</p><!--members-only--><p>PRIVATE-EXPORT-SECRET</p>",
        )
        .await;
    let locked = f
        .post(
            "Private paid article",
            "paid",
            vec![],
            "<p>PRIVATE-PAID-SECRET</p>",
        )
        .await;
    let public = f
        .post(
            "Public article",
            "public",
            vec![],
            "<p>Public full content</p>",
        )
        .await;
    let segmented = f.post("Public with gated card", "public", vec![], "<p>Public part</p><!--kg-gated-block:begin member:true --><p>PRIVATE-CARD-SECRET</p><!--kg-gated-block:end-->").await;
    let (_, cookie) = f.member("paid", &[]).await;
    for identity in [None, Some(cookie.as_str())] {
        let (status, headers, body) = f
            .request(
                "GET",
                &format!("/{}.md", post.slug.as_deref().unwrap()),
                identity,
                Value::Null,
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Safe preview"));
        assert!(!body.contains("PRIVATE-EXPORT-SECRET"));
        assert_eq!(headers["cache-control"], "private, no-store");
        let (status, _, body) = f
            .request(
                "GET",
                &format!("/{}.md", locked.slug.as_deref().unwrap()),
                identity,
                Value::Null,
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(!body.contains("PRIVATE-PAID-SECRET"));
    }
    let (status, _, body) = f
        .request(
            "GET",
            &format!("/{}.md", public.slug.as_deref().unwrap()),
            None,
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Public full content"));
    assert!(!f
        .request(
            "GET",
            &format!("/{}.md", segmented.slug.as_deref().unwrap()),
            None,
            Value::Null
        )
        .await
        .2
        .contains("PRIVATE-CARD-SECRET"));
    for path in ["/", "/rss/", "/llms.txt", &format!("/{}/",segmented.slug.as_deref().unwrap())] {
        let (status, _, body) = f.request("GET", path, None, Value::Null).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(!body.contains("PRIVATE-CARD-SECRET"),"{path} leaked segment-gated content");
        assert!(
            !body.contains("PRIVATE-EXPORT-SECRET"),
            "{path} leaked members body"
        );
        assert!(
            !body.contains("PRIVATE-PAID-SECRET"),
            "{path} leaked paid body"
        );
    }
}

#[tokio::test]
async fn theme_image_urls_resize_and_convert_without_stretching_portraits() {
    let f=Fixture::new().await;
    let directory=std::env::temp_dir().join(format!("ghost-native-images-{}",ghost_core::ids::uuid_v4()));
    std::fs::create_dir_all(directory.join("images")).unwrap();
    let image=image::DynamicImage::ImageRgb8(image::RgbImage::new(600,900));image.save_with_format(directory.join("images/portrait.png"),image::ImageFormat::Png).unwrap();
    let mut state=f.state.clone();let mut config=state.config.as_ref().clone();config.paths.content_path=directory.to_string_lossy().to_string();state.config=Arc::new(config);
    let app=ghost_front::site_router(state);
    let response=app.clone().oneshot(Request::builder().uri("/content/images/size/w200/format/webp/portrait.png").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(),StatusCode::OK);assert_eq!(response.headers()["content-type"],"image/webp");let bytes=to_bytes(response.into_body(),1_000_000).await.unwrap();let image=image::load_from_memory(&bytes).unwrap();assert_eq!((image.width(),image.height()),(200,300));
    let response=app.oneshot(Request::builder().uri("/content/images/size/w999999/portrait.png").body(Body::empty()).unwrap()).await.unwrap();assert_eq!(response.status(),StatusCode::BAD_REQUEST);
    std::fs::remove_dir_all(directory).unwrap();
}
