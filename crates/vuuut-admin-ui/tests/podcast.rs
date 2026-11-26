//! Tools → Podcast end to end, through the real admin and public routers:
//! build a show with hosts, a guest, a public episode with chapters and a
//! transcript and a members-only bonus; switch the podcast on; then read it
//! the way listeners and apps do — the show page, the episode, the RSS
//! feed, chapters, the download counter (deduplicated, bots ignored), a
//! member's private feed — send a guest request, and see it all in the
//! admin. Refusals first.
//!
//! The fixture loads the bundled Atrium theme, which has no `podcast/*.hbs`,
//! so the tool's own markup is what renders.
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
        ("title".into(), serde_json::json!("Podcast test")),
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
    for uri in ["/podcast/", "/podcast/episodes/", "/podcast/shows/", "/podcast/people/", "/podcast/requests/", "/podcast/analytics/", "/podcast/settings/"] {
        let anon = admin(&f.state, "GET", uri, None, None, None).await;
        assert!(anon.status.is_redirection() || anon.status == StatusCode::FORBIDDEN, "{uri}: {}", anon.status);
        assert_ne!(get(&f.state, uri, &author).await.status, StatusCode::OK, "{uri} opened for an Author");
    }
    let cookie = f.session().await;
    let forged = admin(&f.state, "POST", "/podcast/shows/new/", Some(&cookie), Some("https://elsewhere.example"), Some(&[("title", "Nope")])).await;
    assert_eq!(forged.status, StatusCode::FORBIDDEN);
    assert!(vuuut_db::repos::podcast::shows(&f.state.db, false).await.is_empty());
    // Off by default: no public pages, no feed.
    assert_eq!(site(&f.state, "GET", "/podcast/", &[], None).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_show_is_built_published_listened_to_and_reported() {
    let f = fixture().await;
    let cookie = f.session().await;

    // Every screen renders on an empty tool.
    for uri in ["/podcast/", "/podcast/episodes/", "/podcast/shows/", "/podcast/shows/new/", "/podcast/people/", "/podcast/people/new/", "/podcast/sponsors/", "/podcast/requests/", "/podcast/reviews/", "/podcast/analytics/", "/podcast/settings/"] {
        assert_eq!(get(&f.state, uri, &cookie).await.status, StatusCode::OK, "{uri}");
    }
    // A new episode with no show sends you to make the show first.
    assert!(get(&f.state, "/podcast/episodes/new/", &cookie).await.location.contains("/shows/new/"));

    // People, a sponsor, the show.
    let host = post(&f.state, "/podcast/people/new/", &cookie, &[("name", "Ana Host"), ("title", "Host")]).await;
    assert_eq!(host.flash(), "Ana Host added.");
    let guest = post(&f.state, "/podcast/people/new/", &cookie, &[("name", "Gus <Guest>"), ("bio", "Writes about rivers.")]).await;
    assert!(guest.flash().contains("added"));
    let people = vuuut_db::repos::podcast::people(&f.state.db).await;
    let host_id = people.iter().find(|p| p.name == "Ana Host").unwrap().id.clone();
    let guest_id = people.iter().find(|p| p.name.starts_with("Gus")).unwrap().id.clone();
    post(&f.state, "/podcast/sponsors/", &cookie, &[("name", "Acme Tools"), ("url", "https://acme.example"), ("promo_code", "RIVER10"), ("active", "1")]).await;
    let sponsor = vuuut_db::repos::podcast::sponsors(&f.state.db).await[0].id.clone();

    let bad = post(&f.state, "/podcast/shows/new/", &cookie, &[("title", "X"), ("link_spotify", "javascript:alert(1)")]).await;
    assert!(bad.flash().contains("Spotify"), "{}", bad.flash());
    let made = post(&f.state, "/podcast/shows/new/", &cookie, &[
        ("title", "River Talk"), ("subtitle", "Conversations about water"), ("description", "A show about rivers, the people who live by them and the things they carry."),
        ("category", "Science > Earth Sciences"), ("language", "en"), ("author", "Ana Host"), ("owner_email", "owner@example.test"),
        ("link_apple", "https://podcasts.apple.com/river"), ("host", &host_id), (&format!("host_role_{host_id}"), "host"), ("status", "active"), ("tint", "teal"),
        ("image", "https://cdn.example/art.jpg"),
    ]).await;
    assert_eq!(made.flash(), "Show created. Add its first episode next.");
    let show_id = id_from(&made.location, "/shows/");
    let show = vuuut_db::repos::podcast::find_show(&f.state.db, &show_id).await.unwrap();
    assert_eq!(show.slug, "river-talk");
    assert!(!show.guid.is_empty());

    // Publishing needs audio.
    let no_audio = post(&f.state, "/podcast/episodes/new/", &cookie, &[("title", "Empty"), ("show_id", &show_id), ("status", "published")]).await;
    assert!(no_audio.flash().contains("audio"));

    let role_key = format!("role_{guest_id}");
    let ep = post(&f.state, "/podcast/episodes/new/", &cookie, &[
        ("title", "The source <script>alert(1)</script>"), ("show_id", &show_id), ("status", "published"), ("published_at", "2026-01-05T09:00"),
        ("summary", "Where every river starts."), ("audio_url", "https://cdn.example/ep1.mp3"), ("duration", "42:10"), ("audio_bytes", "40000000"),
        ("season", "1"), ("number", "1"), ("episode_type", "full"), ("access", "public"),
        ("ch_time", "0:00"), ("ch_title", "Welcome"), ("ch_url", ""), ("ch_time", "12:30"), ("ch_title", "The spring"), ("ch_url", ""),
        ("transcript", "WEBVTT\n\n00:00:01.000 --> 00:00:04.000\n<v Ana Host>Welcome to River Talk.\n\n00:12:30.000 --> 00:12:35.000\n<v Gus>Every river starts small."),
        ("person", &guest_id), (&role_key, "guest"), ("sponsor", &sponsor), ("notes", "Links: the river atlas."),
    ]).await;
    assert_eq!(ep.flash(), "Episode published.");
    let ep_id = id_from(&ep.location, "/episodes/");
    let episode = vuuut_db::repos::podcast::find_episode(&f.state.db, &ep_id).await.unwrap();
    assert_eq!((episode.duration, episode.season, episode.number), (2530, 1, 1));
    assert_eq!(episode.chapters().len(), 2);
    let bonus = post(&f.state, "/podcast/episodes/new/", &cookie, &[
        ("title", "Members bonus"), ("show_id", &show_id), ("status", "published"), ("published_at", "2026-01-10T09:00"),
        ("audio_url", "https://cdn.example/bonus.mp3"), ("access", "paid"), ("episode_type", "bonus"),
    ]).await;
    let bonus_id = id_from(&bonus.location, "/episodes/");
    // A draft never appears anywhere public.
    post(&f.state, "/podcast/episodes/new/", &cookie, &[("title", "Secret draft"), ("show_id", &show_id), ("status", "draft")]).await;

    // Switch it on.
    let saved = post(&f.state, "/podcast/settings/", &cookie, &[("podcast_enabled", "1"), ("podcast_count_downloads", "1"), ("podcast_requests", "1"), ("podcast_base_path", "podcast"), ("podcast_per_page", "12")]).await;
    assert!(saved.status.is_redirection(), "{}", saved.status);

    // One show: /podcast/ is the show.
    let home = site(&f.state, "GET", "/podcast/", &[], None).await;
    assert_eq!(home.status, StatusCode::OK);
    assert!(home.body.contains("River Talk") && home.body.contains("Apple Podcasts"));
    assert!(home.body.contains("The source &lt;script&gt;"), "titles are escaped");
    assert!(!home.body.contains("<script>alert(1)"));
    assert!(!home.body.contains("Secret draft"));
    assert!(home.body.contains("PodcastSeries"));

    // The episode page: player, chapters, transcript, guest, sponsor.
    let page = site(&f.state, "GET", &format!("/podcast/river-talk/{}/", episode.slug), &[], None).await;
    assert_eq!(page.status, StatusCode::OK);
    for needle in ["data-vp-stage", "data-vp-seek=\"750\"", "The spring", "Every river starts small.", "Gus &lt;Guest&gt;", "RIVER10", "PodcastEpisode"] {
        assert!(page.body.contains(needle), "episode page is missing {needle}");
    }

    // The members-only bonus: summary and a prompt, never the audio.
    let gated = site(&f.state, "GET", "/podcast/river-talk/members-bonus/", &[], None).await;
    assert_eq!(gated.status, StatusCode::OK);
    assert!(gated.body.contains("for paying members"));
    assert!(!gated.body.contains(&format!("/podcast/play/{bonus_id}")));

    // The feed: valid tags, public episodes only, enclosures through the counter.
    let feed = site(&f.state, "GET", "/podcast/river-talk/feed.xml", &[], None).await;
    assert_eq!(feed.status, StatusCode::OK);
    assert!(feed.headers.get("content-type").unwrap().to_str().unwrap().contains("rss+xml"));
    for needle in ["<itunes:category text=\"Science\"><itunes:category text=\"Earth Sciences\"/>", &format!("{ORIGIN}/podcast/play/{ep_id}.mp3"), "<itunes:duration>2530</itunes:duration>",
        "<podcast:chapters", "<podcast:transcript", "role=\"guest\"", "<itunes:owner>", "The source &lt;script&gt;"] {
        assert!(feed.body.contains(needle), "feed is missing {needle}");
    }
    assert!(!feed.body.contains("Members bonus") && !feed.body.contains("Secret draft"));
    let chapters = site(&f.state, "GET", &format!("/podcast/river-talk/{}/chapters.json", episode.slug), &[], None).await;
    assert!(chapters.body.contains("\"startTime\":750"));

    // Downloads: an app twice in a day is one; a bot is none; gated audio is refused.
    let play = format!("/podcast/play/{ep_id}.mp3");
    let first = site(&f.state, "GET", &play, &[("user-agent", "Overcast/3.0 (+http://overcast.fm/)")], None).await;
    assert_eq!(first.status, StatusCode::FOUND);
    assert_eq!(first.location, "https://cdn.example/ep1.mp3");
    site(&f.state, "GET", &play, &[("user-agent", "Overcast/3.0 (+http://overcast.fm/)")], None).await;
    site(&f.state, "GET", &play, &[("user-agent", "Googlebot/2.1")], None).await;
    site(&f.state, "GET", &play, &[("user-agent", "Spotify/8.9 iOS")], None).await;
    let counted = vuuut_db::repos::podcast::find_episode(&f.state.db, &ep_id).await.unwrap().downloads;
    assert_eq!(counted, 2, "one Overcast listener and one Spotify listener");
    let apps = vuuut_db::repos::podcast::downloads_by(&f.state.db, "app", None, "2000-01-01", 10).await;
    assert!(apps.iter().any(|(a, n)| a == "Overcast" && *n == 1));
    assert_eq!(site(&f.state, "GET", &format!("/podcast/play/{bonus_id}.mp3"), &[("user-agent", "Overcast/3.0")], None).await.status, StatusCode::FORBIDDEN);

    // A paying member's private feed has the bonus, and its audio plays with the token.
    let uuid = vuuut_core::ids::uuid_v4();
    sqlx::query("INSERT INTO members (id, uuid, transient_id, email, status, created_at) VALUES (?, ?, ?, 'paid@example.test', 'paid', '2026-01-01 00:00:00')")
        .bind(vuuut_core::ids::new_id()).bind(&uuid).bind(vuuut_core::ids::new_id()).execute(&f.state.db).await.unwrap();
    let (secret, _) = vuuut_db::repos::podcast::config(&f.state.db).await;
    let token = vuuut_core::podcast::feed_token(&secret, &uuid);
    let private = site(&f.state, "GET", &format!("/podcast/private/{token}/river-talk.xml"), &[], None).await;
    assert_eq!(private.status, StatusCode::OK);
    assert!(private.body.contains("Members bonus") && private.body.contains(&format!("?k={token}")));
    assert!(private.body.contains(">yes</podcast:locked>"), "a private feed is always locked");
    let forged = token.replace(&uuid, &vuuut_core::ids::uuid_v4());
    assert_eq!(site(&f.state, "GET", &format!("/podcast/private/{forged}/river-talk.xml"), &[], None).await.status, StatusCode::NOT_FOUND);
    assert_eq!(site(&f.state, "GET", &format!("/podcast/play/{bonus_id}.mp3?k={token}"), &[("user-agent", "Pocket Casts")], None).await.status, StatusCode::FOUND);

    // A guest request: bots get thanks and nothing is stored; a person's is.
    let t_old = (chrono::Utc::now().timestamp_millis() - 60_000).to_string();
    let t_now = chrono::Utc::now().timestamp_millis().to_string();
    let base = [("kind", "guest"), ("name", "Rosa Rivers"), ("email", "rosa@example.test"), ("topic", "Dams"), ("message", "I have built dams for thirty years and would love to talk.")];
    let mut honeypot = base.to_vec();
    honeypot.push(("website_url", "spam"));
    honeypot.push(("t", &t_old));
    site(&f.state, "POST", "/podcast/river-talk/contact/", &[("origin", ORIGIN)], Some(&honeypot)).await;
    let mut fast = base.to_vec();
    fast.push(("t", &t_now));
    site(&f.state, "POST", "/podcast/river-talk/contact/", &[("origin", ORIGIN)], Some(&fast)).await;
    assert_eq!(vuuut_db::repos::podcast::new_requests(&f.state.db).await, 0);
    let mut good = base.to_vec();
    good.push(("t", &t_old));
    let sent = site(&f.state, "POST", "/podcast/river-talk/contact/", &[("origin", ORIGIN)], Some(&good)).await;
    assert!(sent.location.contains("sent=1"), "{}", sent.location);
    assert_eq!(vuuut_db::repos::podcast::new_requests(&f.state.db).await, 1);
    let cross = site(&f.state, "POST", "/podcast/river-talk/contact/", &[("origin", "https://elsewhere.example")], Some(&good)).await;
    assert_eq!(cross.status, StatusCode::FORBIDDEN);

    // The admin sees it all.
    let overview = get(&f.state, "/podcast/", &cookie).await;
    assert!(overview.body.contains("River Talk") && overview.body.contains("Feed check"));
    assert!(get(&f.state, "/podcast/analytics/", &cookie).await.body.contains("Overcast"));
    let inbox = get(&f.state, "/podcast/requests/", &cookie).await;
    assert!(inbox.body.contains("Rosa Rivers") && inbox.body.contains("Dams"));
    for uri in [format!("/podcast/episodes/{ep_id}/"), format!("/podcast/shows/{show_id}/"), format!("/podcast/people/{guest_id}/"), "/podcast/episodes/".to_string()] {
        assert_eq!(get(&f.state, &uri, &cookie).await.status, StatusCode::OK, "{uri}");
    }
    // The guest's page lists the episode.
    let guest_page = site(&f.state, "GET", "/podcast/people/gus-guest/", &[], None).await;
    assert_eq!(guest_page.status, StatusCode::OK);
    assert!(guest_page.body.contains("The source"));

    // Deleting the show takes its episodes and downloads with it.
    let gone = post(&f.state, &format!("/podcast/shows/{show_id}/delete/"), &cookie, &[]).await;
    assert_eq!(gone.flash(), "Show deleted.");
    assert!(vuuut_db::repos::podcast::find_episode(&f.state.db, &ep_id).await.is_none());
    assert_eq!(vuuut_db::repos::podcast::downloads_since(&f.state.db, None, "2000-01-01").await, 0);
}

#[tokio::test]
async fn the_sample_show_loads_once_and_every_page_renders() {
    let f = fixture().await;
    let cookie = f.session().await;
    let loaded = post(&f.state, "/podcast/sample/", &cookie, &[]).await;
    assert!(loaded.status.is_redirection());
    let shows = vuuut_db::repos::podcast::shows(&f.state.db, false).await;
    assert_eq!(shows.len(), 1);
    assert!(vuuut_db::repos::podcast::downloads_since(&f.state.db, None, "2000-01-01").await > 100);
    let again = post(&f.state, "/podcast/sample/", &cookie, &[]).await;
    assert!(again.flash().contains("already"));
    for uri in ["/podcast/", "/podcast/episodes/", "/podcast/analytics/", "/podcast/requests/", "/podcast/reviews/", "/podcast/sponsors/"] {
        assert_eq!(get(&f.state, uri, &cookie).await.status, StatusCode::OK, "{uri}");
    }
    let public = site(&f.state, "GET", "/podcast/", &[], None).await;
    assert_eq!(public.status, StatusCode::OK);
    assert!(public.body.contains("Open Workshop") && public.body.contains("What listeners say"));
    // The episode scheduled for next week is not out yet.
    assert!(!public.body.contains("The business of commissions"));
    let feed = site(&f.state, "GET", "/podcast/open-workshop/feed.xml", &[], None).await;
    assert!(feed.body.contains("<itunes:episodeType>trailer</itunes:episodeType>"));
    assert!(!feed.body.contains("Questions from the workbench"), "the members' bonus stays out of the public feed");
}
