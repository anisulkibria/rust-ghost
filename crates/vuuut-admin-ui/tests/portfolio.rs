//! Tools → Portfolio end to end, through the real admin and public routers:
//! a studio portfolio with a discipline, a client, a service, a published
//! case study and a password-protected one; the public grid, a case study,
//! the lock and the unlock cookie; an enquiry through the contact form
//! (bots, forged budgets and cross-site posts refused); the résumé; and the
//! admin's inbox and CSV. Refusals first.
//!
//! The fixture loads the bundled Atrium theme, which has no
//! `portfolio/*.hbs`, so the tool's own markup is what renders.
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

    async fn author_session(&self) -> String {
        let id = vuuut_core::ids::new_id();
        sqlx::query("INSERT INTO users (id, name, slug, password, email, status, created_at) VALUES (?, 'An author', ?, 'unused', ?, 'active', '2026-11-01 00:00:00')")
            .bind(&id).bind(format!("an-author-{id}")).bind(format!("author-{id}@example.test")).execute(&self.state.db).await.unwrap();
        let role: String = sqlx::query_scalar("SELECT id FROM roles WHERE name = 'Author' LIMIT 1").fetch_one(&self.state.db).await.unwrap();
        sqlx::query("INSERT INTO roles_users (id, role_id, user_id) VALUES (?, ?, ?)").bind(vuuut_core::ids::new_id()).bind(&role).bind(&id).execute(&self.state.db).await.unwrap();
        let session = vuuut_db::repos::sessions::create(&self.state.db, &id).await.unwrap();
        format!("vuuut-admin-session={session}")
    }
}

async fn fixture() -> Fixture {
    let test_db = vuuut_db::pool::connect_test().await.unwrap();
    let db = test_db.pool();
    vuuut_db::seed::seed_if_empty(&db).await.unwrap();
    let owner = vuuut_db::repos::users::setup_owner(&db, "Owner", "owner@example.test", "unused").await.unwrap();
    let settings = Arc::new(vuuut_db::SettingsCache::from_values(vec![
        ("site_url".into(), serde_json::json!(ORIGIN)),
        ("title".into(), serde_json::json!("Portfolio test")),
        ("timezone".into(), serde_json::json!("Etc/UTC")),
    ]));
    let config = serde_json::from_value(serde_json::json!({
        "url": ORIGIN, "server": {}, "database": {"connection": {"filename": ":memory:"}}, "paths": {}
    }))
    .unwrap();
    let routes = vuuut_routing::routes_yaml::parse_routes_yaml(vuuut_routing::routes_yaml::DEFAULT_ROUTES_YAML).unwrap();
    let router = Arc::new(vuuut_routing::RouterTree::build(&routes).unwrap());
    let theme_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/themes/atrium");
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
        redirects: Arc::new(tokio::sync::RwLock::new(Arc::new(vuuut_routing::redirects::Redirects::default()))),
        setup_done: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        frontend_key: Arc::new(tokio::sync::RwLock::new(None)),
        limits: Arc::new(vuuut_db::limits::Limits::default()),
        firewall: vuuut_db::repos::firewall::RuleCache::empty(),
    };
    Fixture { _db: test_db, state, owner: owner.id }
}

struct Reply {
    status: StatusCode,
    location: String,
    headers: axum::http::HeaderMap,
    body: String,
}

impl Reply {
    fn flash(&self) -> String {
        let Some(query) = self.location.split_once('?').map(|(_, q)| q) else { return String::new() };
        form_urlencoded::parse(query.as_bytes()).filter(|(k, _)| k == "ok" || k == "error").map(|(_, v)| v.into_owned()).collect::<Vec<_>>().join(" ")
    }
}

async fn admin(state: &vuuut_api::AppState, method: &str, uri: &str, cookie: Option<&str>, origin: Option<&str>, form: Option<&[(&str, &str)]>) -> Reply {
    let mut request = Request::builder().method(method).uri(uri).header("host", HOST).header("accept", "text/html");
    if let Some(c) = cookie {
        request = request.header("cookie", c);
    }
    if let Some(o) = origin {
        request = request.header("origin", o);
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
    let headers = response.headers().clone();
    let location = headers.get("location").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let bytes = to_bytes(response.into_body(), 20_000_000).await.unwrap();
    Reply { status, location, headers, body: String::from_utf8_lossy(&bytes).into_owned() }
}

async fn get(state: &vuuut_api::AppState, uri: &str, cookie: &str) -> Reply {
    admin(state, "GET", uri, Some(cookie), None, None).await
}

async fn post(state: &vuuut_api::AppState, uri: &str, cookie: &str, pairs: &[(&str, &str)]) -> Reply {
    admin(state, "POST", uri, Some(cookie), Some(ORIGIN), Some(pairs)).await
}

async fn site(state: &vuuut_api::AppState, method: &str, uri: &str, headers: &[(&str, &str)], form: Option<&[(&str, &str)]>) -> Reply {
    let mut req = Request::builder().method(method).uri(uri).header("host", HOST).header("accept", "text/html");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let body = match form {
        Some(pairs) => {
            req = req.header("content-type", "application/x-www-form-urlencoded");
            let mut s = form_urlencoded::Serializer::new(String::new());
            for (k, v) in pairs {
                s.append_pair(k, v);
            }
            Body::from(s.finish())
        }
        None => Body::empty(),
    };
    let response = vuuut_front::site_router(state.clone()).oneshot(req.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let location = headers.get("location").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let bytes = to_bytes(response.into_body(), 20_000_000).await.unwrap();
    Reply { status, location, headers, body: String::from_utf8_lossy(&bytes).into_owned() }
}

fn id_from(location: &str, prefix: &str) -> String {
    location.split(prefix).nth(1).unwrap_or("").split(['/', '?']).next().unwrap_or("").to_string()
}


#[tokio::test]
async fn the_tool_is_closed_to_anyone_who_is_not_signed_in_staff_and_writes_need_the_admin_origin() {
    let f = fixture().await;
    let author = f.author_session().await;
    for uri in ["/portfolio/", "/portfolio/projects/", "/portfolio/services/", "/portfolio/clients/", "/portfolio/enquiries/", "/portfolio/resume/", "/portfolio/settings/"] {
        let anon = admin(&f.state, "GET", uri, None, None, None).await;
        assert!(anon.status.is_redirection() || anon.status == StatusCode::FORBIDDEN, "{uri}: {}", anon.status);
        assert_ne!(get(&f.state, uri, &author).await.status, StatusCode::OK, "{uri} opened for an Author");
    }
    let cookie = f.session().await;
    let forged = admin(&f.state, "POST", "/portfolio/projects/new/", Some(&cookie), Some("https://elsewhere.example"), Some(&[("title", "Nope")])).await;
    assert_eq!(forged.status, StatusCode::FORBIDDEN);
    assert_eq!(site(&f.state, "GET", "/portfolio/", &[], None).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_studio_portfolio_is_built_browsed_unlocked_and_enquired_about() {
    let f = fixture().await;
    let cookie = f.session().await;
    for uri in ["/portfolio/", "/portfolio/projects/", "/portfolio/projects/new/", "/portfolio/services/", "/portfolio/services/new/", "/portfolio/disciplines/", "/portfolio/clients/", "/portfolio/testimonials/", "/portfolio/testimonials/new/", "/portfolio/team/", "/portfolio/team/new/", "/portfolio/resume/", "/portfolio/enquiries/", "/portfolio/settings/"] {
        assert_eq!(get(&f.state, uri, &cookie).await.status, StatusCode::OK, "{uri}");
    }

    let saved = post(&f.state, "/portfolio/settings/", &cookie, &[
        ("portfolio_enabled", "1"), ("portfolio_mode", "studio"), ("portfolio_name", "Harbour Lane"), ("portfolio_headline", "An independent brand studio"),
        ("portfolio_base_path", "work"), ("portfolio_enquiries", "1"), ("portfolio_email", "studio@example.test"), ("portfolio_budgets", "Under $10k\n$10k+"),
        ("portfolio_availability", "available"), ("social_instagram", "https://instagram.com/harbour"), ("portfolio_per_page", "24"),
    ]).await;
    assert!(saved.status.is_redirection(), "{} {}", saved.status, saved.flash());
    let bad = post(&f.state, "/portfolio/settings/", &cookie, &[("social_x", "javascript:alert(1)")]).await;
    assert!(bad.flash().contains("X link"), "{}", bad.flash());

    post(&f.state, "/portfolio/disciplines/", &cookie, &[("name", "Branding"), ("tint", "amber")]).await;
    post(&f.state, "/portfolio/clients/", &cookie, &[("name", "Northfield Coffee"), ("featured", "1")]).await;
    post(&f.state, "/portfolio/services/new/", &cookie, &[("name", "Brand identity"), ("summary", "Marks and systems."), ("price_from", "18000"), ("deliverables", "Logo\nGuidelines"), ("status", "active")]).await;
    let x = vuuut_db::repos::portfolio::Lookup::load(&f.state.db).await;
    let (cat, client, service) = (x.categories[0].id.clone(), x.clients[0].id.clone(), x.services[0].id.clone());

    let made = post(&f.state, "/portfolio/projects/new/", &cookie, &[
        ("title", "Northfield <b>Coffee</b>"), ("subtitle", "A roastery brand"), ("summary", "Identity and packaging."), ("body", "The brief was simple: make mornings feel better."),
        ("image", "https://cdn.example/cover.jpg"), ("image", "https://cdn.example/g1.jpg"), ("client_id", &client), ("year", "2026"), ("role", "Brand"),
        ("category", &cat), ("service", &service), ("m_value", "2×"), ("m_label", "wholesale enquiries"), ("c_name", "Iris Calder"), ("c_role", "Creative director"),
        ("tools", "Figma, Glyphs"), ("status", "published"), ("featured", "1"), ("accent", "#7a4b2a"), ("layout", "case_study"),
    ]).await;
    assert_eq!(made.flash(), "Project created.");
    let needs_pw = post(&f.state, "/portfolio/projects/new/", &cookie, &[("title", "Secret"), ("status", "protected")]).await;
    assert!(needs_pw.flash().contains("password"));
    post(&f.state, "/portfolio/projects/new/", &cookie, &[("title", "Orchard NDA"), ("summary", "Under NDA."), ("body", "Confidential details of the platform."), ("status", "protected"), ("password", "open sesame"), ("category", &cat)]).await;
    post(&f.state, "/portfolio/projects/new/", &cookie, &[("title", "Hidden draft"), ("status", "draft")]).await;
    let nda = vuuut_db::repos::portfolio::project_by_slug(&f.state.db, "orchard-nda").await.unwrap();
    assert!(nda.password_hash.contains('$') && !nda.password_hash.contains("open sesame"));

    // The address moved to /work/; the old one redirects.
    assert!(site(&f.state, "GET", "/portfolio/", &[], None).await.location.ends_with("/work/"));
    let grid = site(&f.state, "GET", "/work/", &[], None).await;
    assert_eq!(grid.status, StatusCode::OK);
    for needle in ["An independent brand studio", "Northfield &lt;b&gt;Coffee&lt;/b&gt;", "Orchard NDA", "vf-lock", "Branding", "Brand identity", "Start a project", "Northfield Coffee"] {
        assert!(grid.body.contains(needle), "grid is missing {needle}");
    }
    assert!(!grid.body.contains("Hidden draft") && !grid.body.contains("<b>Coffee</b>"));
    let one = site(&f.state, "GET", "/work/northfield-b-coffee-b/", &[], None).await;
    assert_eq!(one.status, StatusCode::OK, "{}", one.location);
    for needle in ["make mornings feel better", "wholesale enquiries", "Iris Calder", "--vf-accent:#7a4b2a", "CreativeWork", "g1.jpg"] {
        assert!(one.body.contains(needle), "case study is missing {needle}");
    }
    assert_eq!(site(&f.state, "GET", "/work/hidden-draft/", &[], None).await.status, StatusCode::NOT_FOUND);

    // The protected case study: nothing of it until the password is right.
    let locked = site(&f.state, "GET", "/work/orchard-nda/", &[], None).await;
    assert_eq!(locked.status, StatusCode::OK);
    assert!(locked.body.contains("This case study is private") && !locked.body.contains("Confidential details"));
    assert_eq!(locked.headers.get("x-robots-tag").unwrap(), "noindex");
    let wrong = site(&f.state, "POST", "/work/orchard-nda/unlock/", &[("origin", ORIGIN)], Some(&[("password", "nope")])).await;
    assert!(wrong.location.contains("wrong=1"));
    let right = site(&f.state, "POST", "/work/orchard-nda/unlock/", &[("origin", ORIGIN)], Some(&[("password", "open sesame")])).await;
    let set = right.headers.get("set-cookie").unwrap().to_str().unwrap().to_string();
    assert!(set.starts_with("vuuut-portfolio-unlock=") && set.contains("HttpOnly"));
    let cookie_pair = set.split(';').next().unwrap().to_string();
    let open = site(&f.state, "GET", "/work/orchard-nda/", &[("cookie", &cookie_pair)], None).await;
    assert!(open.body.contains("Confidential details"));
    // A cookie made for another project opens nothing here.
    let forged = cookie_pair.replace(&nda.id, &vuuut_core::ids::new_id());
    assert!(!site(&f.state, "GET", "/work/orchard-nda/", &[("cookie", &forged)], None).await.body.contains("Confidential details"));

    // Services, about.
    let svc = site(&f.state, "GET", "/work/services/brand-identity/", &[], None).await;
    assert!(svc.body.contains("From $18,000") || svc.body.contains("From $18000"), "{}", &svc.body[..svc.body.len().min(0)]);
    assert!(svc.body.contains("Guidelines"));
    assert_eq!(site(&f.state, "GET", "/work/about/", &[], None).await.status, StatusCode::OK);

    // An enquiry.
    let t_old = (chrono::Utc::now().timestamp_millis() - 60_000).to_string();
    let base = [("name", "Ellen Brooks"), ("email", "Ellen@Example.test"), ("company", "Co-op"), ("budget", "Under $10k"), ("services", "Brand identity,Hacking"), ("message", "=HYPERLINK(\"x\") We need a new brand for our second shop.")];
    let mut bot = base.to_vec();
    bot.push(("website_url", "x"));
    bot.push(("t", &t_old));
    site(&f.state, "POST", "/work/contact/", &[("origin", ORIGIN)], Some(&bot)).await;
    assert!(vuuut_db::repos::portfolio::enquiries(&f.state.db, None, 10).await.is_empty());
    let mut good = base.to_vec();
    good.push(("t", &t_old));
    let sent = site(&f.state, "POST", "/work/contact/", &[("origin", ORIGIN)], Some(&good)).await;
    assert!(sent.location.contains("sent=1"), "{}", sent.location);
    let enquiries = vuuut_db::repos::portfolio::enquiries(&f.state.db, None, 10).await;
    assert_eq!(enquiries.len(), 1);
    assert_eq!((enquiries[0].email.as_str(), enquiries[0].budget.as_str(), enquiries[0].services.as_str()), ("ellen@example.test", "Under $10k", "Brand identity"));
    let mut forged_budget = good.clone();
    forged_budget.retain(|(k, _)| *k != "budget");
    forged_budget.push(("budget", "$1,000,000"));
    forged_budget.retain(|(k, _)| *k != "email");
    forged_budget.push(("email", "other@example.test"));
    site(&f.state, "POST", "/work/contact/", &[("origin", ORIGIN)], Some(&forged_budget)).await;
    assert!(vuuut_db::repos::portfolio::enquiries(&f.state.db, None, 10).await.iter().all(|e| e.budget != "$1,000,000"));
    assert_eq!(site(&f.state, "POST", "/work/contact/", &[("origin", "https://elsewhere.example")], Some(&good)).await.status, StatusCode::FORBIDDEN);

    // The admin: overview, inbox, detail, CSV with the formula defused.
    assert!(get(&f.state, "/portfolio/", &cookie).await.body.contains("Getting it right"));
    let inbox = get(&f.state, "/portfolio/enquiries/", &cookie).await;
    assert!(inbox.body.contains("Ellen Brooks"));
    let detail = get(&f.state, &format!("/portfolio/enquiries/{}/", enquiries[0].id), &cookie).await;
    assert!(detail.body.contains("second shop"));
    let moved = post(&f.state, &format!("/portfolio/enquiries/{}/", enquiries[0].id), &cookie, &[("status", "won"), ("note", "Signed.")]).await;
    assert_eq!(moved.flash(), "Saved.");
    let csv = get(&f.state, "/portfolio/enquiries/export.csv", &cookie).await;
    assert!(csv.body.contains("\"won\"") && !csv.body.contains(",\"=HYPERLINK"));

    // The résumé.
    let entry = post(&f.state, "/portfolio/resume/entries/", &cookie, &[("kind", "award"), ("title", "Gold, Brand Identity"), ("organisation", "Lisbon Design Awards"), ("start", "2025")]).await;
    assert_eq!(entry.flash(), "Added.");
    let backwards = post(&f.state, "/portfolio/resume/entries/", &cookie, &[("title", "Designer"), ("start", "2024"), ("end", "2020")]).await;
    assert!(backwards.flash().contains("before"));
    post(&f.state, "/portfolio/resume/skills/", &cookie, &[("name", "Typography"), ("group", "Design"), ("level", "90")]).await;
    let resume = site(&f.state, "GET", "/work/resume/", &[], None).await;
    assert!(resume.body.contains("Gold, Brand Identity") && resume.body.contains("Typography"));

    // Theme data: {{#get "projects"}} never hands out a locked project's body.
    let x = vuuut_db::repos::portfolio::Lookup::load(&f.state.db).await;
    let s = vuuut_core::portfolio::Settings::read(|k| f.state.settings.get_str(k));
    let j = vuuut_db::repos::portfolio::project_json(&s, &nda, &x, false);
    assert!(j["body"].is_null() && j["locked"] == true);
}

#[tokio::test]
async fn both_samples_load_and_every_page_renders() {
    for kind in ["personal", "studio"] {
        let f = fixture().await;
        let cookie = f.session().await;
        let loaded = post(&f.state, "/portfolio/sample/", &cookie, &[("kind", kind)]).await;
        assert!(loaded.status.is_redirection(), "{kind}");
        assert!(post(&f.state, "/portfolio/sample/", &cookie, &[("kind", kind)]).await.flash().contains("already"));
        for uri in ["/portfolio/", "/portfolio/projects/", "/portfolio/resume/", "/portfolio/team/", "/portfolio/enquiries/", "/portfolio/clients/", "/portfolio/testimonials/"] {
            assert_eq!(get(&f.state, uri, &cookie).await.status, StatusCode::OK, "{kind} {uri}");
        }
        let home = site(&f.state, "GET", "/portfolio/", &[], None).await;
        assert_eq!(home.status, StatusCode::OK, "{kind}");
        assert!(home.body.contains(if kind == "studio" { "Harbour Lane" } else { "Lena Hart" }) || home.body.contains(if kind == "studio" { "independent brand" } else { "calm, useful software" }));
        for uri in ["/portfolio/about/", "/portfolio/services/", "/portfolio/contact/"] {
            assert_eq!(site(&f.state, "GET", uri, &[], None).await.status, StatusCode::OK, "{kind} {uri}");
        }
        if kind == "personal" {
            assert_eq!(site(&f.state, "GET", "/portfolio/resume/", &[], None).await.status, StatusCode::OK);
        } else {
            assert_eq!(site(&f.state, "GET", "/portfolio/team/iris-calder/", &[], None).await.status, StatusCode::OK);
        }
    }
}
