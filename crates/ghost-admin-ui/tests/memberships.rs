//! The Memberships module end to end: who is refused, what the overview says
//! about a site, and whether the tier and offer screens tell the truth about
//! what a visitor can reach.
//!
//! The negative tests come first, and there are two kinds:
//!
//! * **Refusals** — the overview names members with their email addresses and
//!   states what the site earns, so the first test is the one that says a
//!   Contributor, an Author and an Editor cannot have any of it.
//! * **Judgements that must not be flattering** — a seeded site looks fine on
//!   every settings screen it has and sells nothing at all: its only paid tier
//!   is hidden, Stripe is not connected, and the portal offers no plan. A
//!   report that called that healthy would be worse than no report, so several
//!   of these tests assert on what the screen refuses to claim.
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
        ("title".into(), serde_json::json!("Memberships test")),
    ];
    values.extend(extra_settings);
    let settings = Arc::new(ghost_db::SettingsCache::from_values(values));
    let config = serde_json::from_value(serde_json::json!({
        "url": ORIGIN, "server": {},
        "database": {"connection": {"filename": ":memory:"}},
        "paths": {"contentPath": std::env::temp_dir()
            .join(format!("ghost-memberships-test-{}", ghost_core::ids::new_id()))
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
        firewall: ghost_db::repos::firewall::RuleCache::empty(),
    };
    (test_db, state, owner.id)
}

struct Reply {
    status: StatusCode,
    location: String,
    body: String,
}

async fn admin(state: &ghost_api::AppState, uri: &str, cookie: Option<&str>) -> Reply {
    let mut request = Request::builder()
        .method("GET")
        .uri(uri)
        .header("host", HOST)
        .header("accept", "text/html");
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
    }
    let app = ghost_admin_ui::admin_ui_router(state.clone()).with_state(state.clone());
    let response = app.oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
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

fn cookie_for(sid: &str) -> String {
    format!("vuuut-admin-session={sid}")
}

async fn owner_cookie(state: &ghost_api::AppState, owner: &str) -> String {
    cookie_for(&session_for(state, owner).await)
}

/// A member, with the status they are on and how long ago they joined.
async fn member(state: &ghost_api::AppState, name: &str, email: &str, status: &str, days: i64) -> String {
    let id = ghost_core::ids::new_id();
    let created =
        ghost_core::dates::to_db_string(&(chrono::Utc::now() - chrono::Duration::days(days)));
    sqlx::query(
        "INSERT INTO members (id, uuid, transient_id, email, status, name, email_disabled, \
         created_at) VALUES (?, ?, ?, ?, ?, ?, 0, ?)",
    )
    .bind(&id)
    .bind(ghost_core::ids::uuid_v4())
    .bind(ghost_core::ids::new_id())
    .bind(email)
    .bind(status)
    .bind(name)
    .bind(&created)
    .execute(&state.db)
    .await
    .unwrap();
    id
}

/// Make the seeded paid tier visible, which is what connecting a shop means.
async fn reveal_default_tier(state: &ghost_api::AppState) {
    sqlx::query("UPDATE products SET visibility = 'public' WHERE type = 'paid'")
        .execute(&state.db)
        .await
        .unwrap();
}

const SCREENS: &[&str] =
    &["/memberships/", "/memberships/tiers/", "/memberships/offers/"];

// ---------------------------------------------------------------------------
// refusals
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_module_is_closed_to_anyone_who_is_not_a_signed_in_administrator() {
    let (_db, state, _owner) = fixture().await;
    for uri in SCREENS {
        for cookie in [None, Some("vuuut-admin-session=not-a-session")] {
            let r = admin(&state, uri, cookie).await;
            assert!(
                r.status.is_redirection() && r.location.contains("signin")
                    || r.status == StatusCode::UNAUTHORIZED,
                "GET {uri} answered {} to a stranger",
                r.status
            );
            // Nothing from the report leaks into a refusal page.
            assert!(!r.body.contains("mb-tile-value"), "{uri} showed the funnel to a stranger");
            assert!(!r.body.contains("Latest arrivals"), "{uri} named members to a stranger");
        }
    }
}

#[tokio::test]
async fn a_contributor_an_author_and_an_editor_are_all_refused() {
    // The overview holds readers' email addresses and the site's revenue.
    // Neither is any of these three roles' business, and Editor is the one
    // worth naming: it can edit everybody's posts and still may not see this.
    let (_db, state, _owner) = fixture().await;
    for (i, role) in ["Contributor", "Author", "Editor"].iter().enumerate() {
        let user = staff(&state, &format!("Staff{i}"), &format!("staff{i}@example.test"), role).await;
        let cookie = cookie_for(&session_for(&state, &user).await);
        for uri in SCREENS {
            let r = admin(&state, uri, Some(&cookie)).await;
            assert!(
                r.status.is_redirection() || r.status == StatusCode::UNAUTHORIZED
                    || r.body.contains("Not allowed"),
                "{role} opened {uri} and got {}",
                r.status
            );
            assert!(!r.body.contains("mb-tile-value"), "{role} saw the funnel on {uri}");
        }
    }
}

// ---------------------------------------------------------------------------
// the module card and the rail
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_modules_hub_carries_a_memberships_card() {
    let (_db, state, owner) = fixture().await;
    let cookie = owner_cookie(&state, &owner).await;
    let r = admin(&state, "/tools/", Some(&cookie)).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains(r#"href="/ghost/memberships/""#), "no Memberships card on the hub");
    assert!(r.body.contains("<h2>Memberships</h2>"));
}

#[tokio::test]
async fn members_left_the_rail_and_every_screen_of_the_module_lights_modules() {
    // The transfer, asserted at the only place it is visible: the rail.
    let (_db, state, owner) = fixture().await;
    let cookie = owner_cookie(&state, &owner).await;
    for uri in SCREENS.iter().chain(["/members/"].iter()) {
        let r = admin(&state, uri, Some(&cookie)).await;
        assert_eq!(r.status, StatusCode::OK, "{uri}");
        assert!(
            r.body.contains(r#"class="nav-item active" href="/ghost/tools/""#),
            "{uri} does not light Tools in the rail"
        );
        assert!(
            !r.body.contains(r#"class="nav-item" href="/ghost/members/""#),
            "{uri} still shows a Members rail item"
        );
    }
}

#[tokio::test]
async fn the_members_list_keeps_its_url_and_leads_back_up_to_the_module() {
    // Thirty-odd links in this admin point at `/ghost/members/`, as does
    // every bookmark anybody has made. The module reorganisation must not
    // have moved it.
    let (_db, state, owner) = fixture().await;
    let cookie = owner_cookie(&state, &owner).await;
    let r = admin(&state, "/members/", Some(&cookie)).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("<h1>Members</h1>"));
    assert!(r.body.contains(r#"href="/ghost/memberships/""#), "no way back up to the module");
    // And the module's tab bar points at it rather than at a copy.
    let overview = admin(&state, "/memberships/", Some(&cookie)).await;
    assert!(overview.body.contains(r#"class="seg" href="/ghost/members/""#));
}

// ---------------------------------------------------------------------------
// the overview
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_seeded_site_is_told_it_sells_nothing_rather_than_told_it_is_fine() {
    // A fresh install looks correct on every settings screen it has: there is
    // an active paid tier, there is a newsletter, signups are open. It also
    // cannot take a penny — the tier is hidden, Stripe is absent and the
    // portal has no plan selected. This is the case the module exists for.
    let (_db, state, owner) = fixture().await;
    let cookie = owner_cookie(&state, &owner).await;
    let r = admin(&state, "/memberships/", Some(&cookie)).await;
    assert_eq!(r.status, StatusCode::OK);

    assert!(r.body.contains("gap-tiers-hidden"), "the hidden paid tier went unreported");
    assert!(r.body.contains("gap-stripe-missing"), "the missing processor went unreported");
    assert!(r.body.contains("gap-portal-no-plans"), "the empty portal went unreported");
    assert!(r.body.contains("Blocking"));
    // And it does not claim the site is in good order.
    assert!(!r.body.contains("Nothing is in the way of a signup"));
}

#[tokio::test]
async fn the_funnel_counts_what_is_there_and_names_the_window() {
    let (_db, state, owner) = fixture().await;
    member(&state, "Ann", "ann@example.test", "free", 2).await;
    member(&state, "Bo", "bo@example.test", "free", 3).await;
    member(&state, "Cal", "cal@example.test", "paid", 4).await;
    member(&state, "Dee", "dee@example.test", "comped", 400).await;
    let cookie = owner_cookie(&state, &owner).await;

    let r = admin(&state, "/memberships/", Some(&cookie)).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("4 members"), "the hero does not count the members");
    assert!(r.body.contains("2 free \u{b7} 1 comped"), "the status split is wrong");
    // Three joined inside 30 days; the fourth is over a year old.
    assert!(r.body.contains("Joined in 30 days"));
    // 2 of 4 pay or are comped.
    assert!(r.body.contains("50.0%"), "the conversion rate is wrong");
}

#[tokio::test]
async fn an_empty_site_says_so_instead_of_reporting_a_zero_per_cent_conversion() {
    // `0.0%` says the site tried and failed; a dash says there is nothing to
    // divide by. They are different statements and only one is true here.
    let (_db, state, owner) = fixture().await;
    let cookie = owner_cookie(&state, &owner).await;
    let r = admin(&state, "/memberships/", Some(&cookie)).await;
    assert!(r.body.contains("Nobody has joined yet"));
    assert!(r.body.contains("gap-no-members"));
    assert!(!r.body.contains("0.0%"), "an empty site was given a conversion rate");
}

#[tokio::test]
async fn a_member_chose_their_own_name_so_the_overview_escapes_it() {
    let (_db, state, owner) = fixture().await;
    member(&state, r#"<img src=x onerror=alert(1)>"#, "hostile@example.test", "free", 1).await;
    let cookie = owner_cookie(&state, &owner).await;
    let r = admin(&state, "/memberships/", Some(&cookie)).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(!r.body.contains("<img src=x"), "a member's name reached the page as markup");
    assert!(r.body.contains("&lt;img src=x"), "the name is missing rather than escaped");
}

#[tokio::test]
async fn a_shop_that_works_is_told_nothing_is_in_the_way() {
    // The other end of the first test: the same seeded site with its tier
    // revealed, Stripe connected, a plan in the portal, tips on and comments
    // on has nothing left to report.
    let (_db, state, owner) = fixture_with(vec![
        ("stripe_secret_key".into(), serde_json::json!("sk_test_x")),
        ("portal_plans".into(), serde_json::json!(["free", "monthly", "yearly"])),
        ("comments_enabled".into(), serde_json::json!("all")),
        ("donations_suggested_amount".into(), serde_json::json!(500)),
    ])
    .await;
    reveal_default_tier(&state).await;
    for i in 0..3 {
        member(&state, &format!("M{i}"), &format!("m{i}@example.test"), "paid", 2).await;
    }
    // An offer, so the "no offers running" note does not stand either.
    let tier: String = sqlx::query_scalar("SELECT id FROM products WHERE type = 'paid' LIMIT 1")
        .fetch_one(&state.db)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO offers (id, active, name, code, product_id, interval, currency, \
         discount_type, discount_amount, duration, portal_title, redemption_type, created_at) \
         VALUES (?, 1, 'Spring', 'spring', ?, 'month', 'usd', 'percent', 20, 'once', 'Spring', 'multiple', ?)",
    )
    .bind(ghost_core::ids::new_id())
    .bind(&tier)
    .bind(ghost_core::dates::now_db())
    .execute(&state.db)
    .await
    .unwrap();

    let cookie = owner_cookie(&state, &owner).await;
    let r = admin(&state, "/memberships/", Some(&cookie)).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(
        r.body.contains("Nothing is in the way of a signup"),
        "a working shop was still reported as broken"
    );
    assert!(!r.body.contains("Blocking"));
}

#[tokio::test]
async fn closing_signups_is_reported_and_stops_the_shop_being_nagged_about() {
    let (_db, state, owner) = fixture_with(vec![(
        "members_signup_access".into(),
        serde_json::json!("none"),
    )])
    .await;
    let cookie = owner_cookie(&state, &owner).await;
    let r = admin(&state, "/memberships/", Some(&cookie)).await;
    assert!(r.body.contains("gap-signups-closed"));
    // A site with its doors shut is not a site with a broken shop.
    assert!(!r.body.contains("gap-stripe-missing"), "a closed site was told to connect Stripe");
    assert!(!r.body.contains("gap-portal-no-plans"));
}

#[tokio::test]
async fn the_window_is_reduced_to_one_the_screen_offers() {
    let (_db, state, owner) = fixture().await;
    let cookie = owner_cookie(&state, &owner).await;
    for (asked, shown) in [("7", "7 days"), ("90", "90 days")] {
        let r = admin(&state, &format!("/memberships/?days={asked}"), Some(&cookie)).await;
        assert_eq!(r.status, StatusCode::OK);
        assert!(r.body.contains(&format!("Joined in {} days", &shown[..shown.len() - 5])), "{asked}");
    }
    // Anything else collapses to 30 rather than being believed.
    for forged in ["4000", "-1", "'; DROP TABLE members; --", ""] {
        let r = admin(
            &state,
            &format!("/memberships/?days={}", urlencode(forged)),
            Some(&cookie),
        )
        .await;
        assert_eq!(r.status, StatusCode::OK, "{forged}");
        assert!(r.body.contains("Joined in 30 days"), "{forged} was not reduced to the default");
    }
}

fn urlencode(raw: &str) -> String {
    form_urlencoded::Serializer::new(String::new()).append_pair("v", raw).finish()[2..].to_string()
}

#[tokio::test]
async fn where_members_came_from_is_attributed_and_the_unattributable_are_admitted_to() {
    let (_db, state, owner) = fixture().await;
    let one = member(&state, "Ann", "ann@example.test", "free", 1).await;
    // Arrived with no event row, as an old import leaves them.
    member(&state, "Bo", "bo@example.test", "free", 1).await;
    sqlx::query("INSERT INTO members_created_events (id, member_id, source, created_at) VALUES (?, ?, 'member', ?)")
        .bind(ghost_core::ids::new_id())
        .bind(&one)
        .bind(ghost_core::dates::now_db())
        .execute(&state.db)
        .await
        .unwrap();

    let cookie = owner_cookie(&state, &owner).await;
    let r = admin(&state, "/memberships/", Some(&cookie)).await;
    assert!(r.body.contains("Where they came from"));
    assert!(r.body.contains("The site"), "a portal signup was not attributed");
    assert!(
        r.body.contains("have no recorded source"),
        "the unattributable member was silently dropped"
    );
}

#[tokio::test]
async fn the_newsletter_card_counts_confirmed_subscribers_only() {
    let (_db, state, owner) = fixture().await;
    let m = member(&state, "Ann", "ann@example.test", "free", 1).await;
    let other = member(&state, "Bo", "bo@example.test", "free", 1).await;
    let news: String = sqlx::query_scalar("SELECT id FROM newsletters LIMIT 1")
        .fetch_one(&state.db)
        .await
        .unwrap();
    for (who, status) in [(&m, "confirmed"), (&other, "pending")] {
        sqlx::query("INSERT INTO members_newsletters (id, member_id, newsletter_id, status) VALUES (?, ?, ?, ?)")
            .bind(ghost_core::ids::new_id())
            .bind(who)
            .bind(&news)
            .bind(status)
            .execute(&state.db)
            .await
            .unwrap();
    }

    let cookie = owner_cookie(&state, &owner).await;
    let r = admin(&state, "/memberships/", Some(&cookie)).await;
    assert!(r.body.contains("1 subscriber"), "an unconfirmed subscription was counted as reach");
}

// ---------------------------------------------------------------------------
// tiers
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_tiers_screen_says_a_hidden_paid_tier_is_not_on_sale() {
    let (_db, state, owner) = fixture().await;
    let cookie = owner_cookie(&state, &owner).await;
    let r = admin(&state, "/memberships/tiers/", Some(&cookie)).await;
    assert_eq!(r.status, StatusCode::OK);
    // The seeded pair: Free (public) and Default Product (hidden).
    assert!(r.body.contains("Default Product"));
    assert!(r.body.contains(">Hidden<"), "the hidden tier was not marked hidden");
    assert!(r.body.contains("never shown a way to pay"));
    assert!(!r.body.contains(">On sale<"));

    reveal_default_tier(&state).await;
    let r = admin(&state, "/memberships/tiers/", Some(&cookie)).await;
    assert!(r.body.contains(">On sale<"), "a revealed tier is still not on sale");
    assert!(r.body.contains("$5.00 a month"));
}

#[tokio::test]
async fn an_inactive_tier_is_still_listed_so_it_can_be_switched_back_on() {
    let (_db, state, owner) = fixture().await;
    sqlx::query("UPDATE products SET active = 0 WHERE type = 'paid'")
        .execute(&state.db)
        .await
        .unwrap();
    let cookie = owner_cookie(&state, &owner).await;
    let r = admin(&state, "/memberships/tiers/", Some(&cookie)).await;
    assert!(r.body.contains("Default Product"), "a switched-off tier vanished from its own screen");
    assert!(r.body.contains(">Off<"));
    // The verdict counts the tier as existing and as not for sale, which is
    // the honest reading: there is something to turn back on.
    assert!(
        r.body.contains("none of them is both active and visible"),
        "the site was not told it sells nothing"
    );
}

#[tokio::test]
async fn a_tier_row_links_to_the_editor_that_already_owned_it() {
    let (_db, state, owner) = fixture().await;
    let tier: String = sqlx::query_scalar("SELECT id FROM products WHERE type = 'paid' LIMIT 1")
        .fetch_one(&state.db)
        .await
        .unwrap();
    let cookie = owner_cookie(&state, &owner).await;
    let r = admin(&state, "/memberships/tiers/", Some(&cookie)).await;
    assert!(r.body.contains(&format!("/ghost/settings/memberships/tiers/{tier}/")));
    // And the create button posts to the handler that has always made tiers.
    assert!(r.body.contains(r#"action="/ghost/settings/memberships/tiers/new/""#));
}

// ---------------------------------------------------------------------------
// offers
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_offers_screen_is_empty_before_there_is_an_offer_and_says_why() {
    let (_db, state, owner) = fixture().await;
    let cookie = owner_cookie(&state, &owner).await;
    let r = admin(&state, "/memberships/offers/", Some(&cookie)).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("No offers yet"));
    assert!(r.body.contains("discounts a paid tier"));
    assert!(r.body.contains(r#"action="/ghost/growth/offers/new/""#));
}

#[tokio::test]
async fn an_offer_reports_its_discount_its_tier_and_its_redemptions() {
    let (_db, state, owner) = fixture().await;
    let tier: String = sqlx::query_scalar("SELECT id FROM products WHERE type = 'paid' LIMIT 1")
        .fetch_one(&state.db)
        .await
        .unwrap();
    let offer = ghost_core::ids::new_id();
    sqlx::query(
        "INSERT INTO offers (id, active, name, code, product_id, interval, currency, \
         discount_type, discount_amount, duration, portal_title, redemption_type, created_at) \
         VALUES (?, 1, 'Spring sale', 'spring', ?, 'year', 'usd', 'percent', 25, 'forever', 'Spring', 'multiple', ?)",
    )
    .bind(&offer)
    .bind(&tier)
    .bind(ghost_core::dates::now_db())
    .execute(&state.db)
    .await
    .unwrap();
    let m = member(&state, "Ann", "ann@example.test", "paid", 1).await;
    sqlx::query("UPDATE members SET offer_id = ? WHERE id = ?")
        .bind(&offer)
        .bind(&m)
        .execute(&state.db)
        .await
        .unwrap();

    let cookie = owner_cookie(&state, &owner).await;
    let r = admin(&state, "/memberships/offers/", Some(&cookie)).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("Spring sale"));
    assert!(r.body.contains("25% off every year"), "the discount is misdescribed");
    assert!(r.body.contains("Default Product"), "the offer does not name its tier");
    assert!(r.body.contains("1 redemption"));
    assert!(r.body.contains(">Running<"));
    assert!(r.body.contains(&format!("/ghost/growth/offers/{offer}/")));
}

#[tokio::test]
async fn the_offer_code_is_shown_escaped_rather_than_trusted() {
    let (_db, state, owner) = fixture().await;
    let tier: String = sqlx::query_scalar("SELECT id FROM products WHERE type = 'paid' LIMIT 1")
        .fetch_one(&state.db)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO offers (id, active, name, code, product_id, interval, currency, \
         discount_type, discount_amount, duration, portal_title, redemption_type, created_at) \
         VALUES (?, 1, ?, ?, ?, 'month', 'usd', 'percent', 20, 'once', 'x', 'multiple', ?)",
    )
    .bind(ghost_core::ids::new_id())
    .bind(r#"<script>alert(1)</script>"#)
    .bind(r#"a"onmouseover="x"#)
    .bind(&tier)
    .bind(ghost_core::dates::now_db())
    .execute(&state.db)
    .await
    .unwrap();

    let cookie = owner_cookie(&state, &owner).await;
    let r = admin(&state, "/memberships/offers/", Some(&cookie)).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(!r.body.contains("<script>alert(1)</script>"));
    assert!(!r.body.contains(r#"onmouseover="x"#));
}
