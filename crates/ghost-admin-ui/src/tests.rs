//! Tests for the admin's shared chrome and for screens whose handlers
//! live in sibling modules. Declared from the crate root, so `super::`
//! here means the root, as it did when these sat inside lib.rs.

use super::{member_batch_bar, merge_night_shift, night_shift, page_for, recommendation_title, recommendation_url, rail};

/// The same trap the posts bar had: the remove-label radios are
/// `required` and share the form with every other action, so without
/// `formnovalidate` Unsubscribe and Delete are silently refused whenever
/// no label is picked — which is always, until that popover is opened.
#[test]
fn only_the_remove_label_button_waits_for_the_label_radios() {
    let bar = member_batch_bar(&["vip".to_string()]);
    assert!(bar.contains(r#"name="label" value="vip" required"#), "{bar}");
    for action in ["add_label", "unsubscribe", "delete"] {
        assert!(
            bar.contains(&format!(r#"formnovalidate name="action" value="{action}""#)),
            "{action} would be blocked by the label radios: {bar}"
        );
    }
    assert!(bar.contains(r#"type="submit" name="action" value="remove_label""#), "{bar}");
}

/// A user with the given accessibility blob, the way a signed-in page
/// would see them.
fn user_with(accessibility: Option<&str>) -> ghost_core::models::User {
    let mut user = ghost_core::models::User::default();
    user.accessibility = accessibility.map(str::to_string);
    user
}

/// Ghost's own rules for the stored preference: absent and unreadable are
/// light (not the OS), the pre-string boolean shape still reads, and only
/// the three real choices survive.
#[test]
fn night_shift_reads_ghosts_shapes_and_defaults_to_light() {
    assert_eq!(night_shift(None), "light");
    assert_eq!(night_shift(Some(&user_with(None))), "light");
    assert_eq!(night_shift(Some(&user_with(Some("not json")))), "light");
    assert_eq!(night_shift(Some(&user_with(Some(r#"{}"#)))), "light");
    assert_eq!(night_shift(Some(&user_with(Some(r#"{"nightShift":"neon"}"#)))), "light");
    assert_eq!(night_shift(Some(&user_with(Some(r#"{"nightShift":"dark"}"#)))), "dark");
    assert_eq!(night_shift(Some(&user_with(Some(r#"{"nightShift":"system"}"#)))), "system");
    // the shape Ghost migrated away from
    assert_eq!(night_shift(Some(&user_with(Some(r#"{"nightShift":true}"#)))), "dark");
    assert_eq!(night_shift(Some(&user_with(Some(r#"{"nightShift":false}"#)))), "light");
}

/// The blob is shared with Ghost, so a save must only ever touch its own
/// key — both directions: keep what is there, and start one when the
/// column has never been written.
#[test]
fn merging_night_shift_leaves_the_rest_of_the_blob_alone() {
    assert_eq!(merge_night_shift(None, "dark"), r#"{"nightShift":"dark"}"#);
    let merged = merge_night_shift(Some(r#"{"nightShift":"light","whatsNew":"1.0"}"#), "system");
    let parsed: serde_json::Value = serde_json::from_str(&merged).unwrap();
    assert_eq!(parsed["nightShift"], "system");
    assert_eq!(parsed["whatsNew"], "1.0", "{merged}");
}

/// The document has to open already settled: a dark user gets the class
/// from the server, everything else carries its choice as data-theme for
/// the head script, and the rail offers all three options with the right
/// one marked.
#[test]
fn the_page_opens_on_the_user_s_scheme_and_offers_to_change_it() {
    let dark = page_for("Posts", "posts", Some(&user_with(Some(r#"{"nightShift":"dark"}"#))), super::Role::Contributor, "", true).0;
    assert!(dark.contains(r#"<html class="dark" data-theme="dark">"#), "{dark}");
    assert!(dark.contains(r#"class="menu-item on" data-choice="dark""#), "{dark}");

    let light = page_for("Posts", "posts", Some(&user_with(Some(r#"{"nightShift":"light"}"#))), super::Role::Contributor, "", true).0;
    assert!(light.contains(r#"<html data-theme="light">"#), "{light}");

    let system = page_for("Posts", "posts", Some(&user_with(Some(r#"{"nightShift":"system"}"#))), super::Role::Contributor, "", true).0;
    assert!(system.contains(r#"<html data-theme="system">"#), "{system}");
    // no preference yet: Ghost's default is light, never the OS
    let fresh = page_for("Posts", "posts", Some(&user_with(None)), super::Role::Contributor, "", true).0;
    assert!(fresh.contains(r#"<html data-theme="light">"#), "{fresh}");

    // the switcher itself: three choices, saving where the handler lives
    let menu = rail("posts", Some(&user_with(Some(r#"{"nightShift":"system"}"#))), super::Role::Contributor);
    assert!(menu.contains(r#"data-save="/ghost/appearance/""#), "{menu}");
    assert_eq!(menu.matches("data-choice=").count(), 3, "{menu}");
    assert!(menu.contains(r#"class="menu-item on" data-choice="system""#), "{menu}");
}

#[tokio::test]
async fn tier_newsletters_require_a_current_matching_entitlement() {
    let test_db = ghost_db::pool::connect_test().await.unwrap();
    let pool = test_db.pool();
    ghost_db::seed::seed_if_empty(&pool).await.unwrap();
    let owner = ghost_db::repos::users::setup_owner(&pool, "Test owner", "owner@example.test", "unused-test-hash")
        .await.unwrap();
    sqlx::query(ghost_db::dialect::sql("INSERT INTO newsletters (id, name, slug, created_at) VALUES ('newsletter', 'Test', 'test', datetime('now'))"))
        .execute(&pool).await.unwrap();
    for id in ["selected", "other"] {
        sqlx::query(ghost_db::dialect::sql("INSERT INTO products (id, name, slug, created_at) VALUES (?, ?, ?, datetime('now'))"))
            .bind(id).bind(id).bind(id).execute(&pool).await.unwrap();
    }
    let post = ghost_db::repos::posts::insert(&pool, &ghost_db::repos::posts::NewPost {
        title: "Tier newsletter".into(), status: "published".into(),
        visibility: "tiers".into(), post_type: "post".into(),
        tier_ids: vec!["selected".into()], ..Default::default()
    }, &owner.id).await.unwrap();
    for (id, status, tier, expiry, disabled, subscribed) in [
        ("matching", "paid", "selected", None, false, true),
        ("comped", "comped", "selected", None, false, true),
        ("wrong", "paid", "other", None, false, true),
        ("expired", "paid", "selected", Some("2000-01-01"), false, true),
        ("free", "free", "selected", None, false, true),
        ("disabled", "paid", "selected", None, true, true),
        ("unsubscribed", "paid", "selected", None, false, false),
    ] {
        sqlx::query(ghost_db::dialect::sql("INSERT INTO members (id, transient_id, email, status, email_disabled, created_at) VALUES (?, ?, ?, ?, ?, datetime('now'))"))
            .bind(id).bind(id).bind(format!("{id}@example.test")).bind(status).bind(disabled)
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO members_products (id, member_id, product_id, expiry_at) VALUES (?, ?, ?, ?)")
            .bind(id).bind(id).bind(tier).bind(expiry).execute(&pool).await.unwrap();
        if subscribed {
            sqlx::query("INSERT INTO members_newsletters (id, member_id, newsletter_id) VALUES (?, ?, 'newsletter')")
                .bind(id).bind(id).execute(&pool).await.unwrap();
        }
    }
    let mut emails: Vec<_> = super::newsletter_recipients(&pool, "newsletter", "tiers", &post.id)
        .await.into_iter().map(|member| member.email).collect();
    emails.sort();
    assert_eq!(emails, ["comped@example.test", "matching@example.test"]);
    assert!(super::newsletter_recipients(&pool, "newsletter", "tiers", "missing-post").await.is_empty());
}

#[tokio::test]
async fn the_newsletters_tab_counts_what_sending_recorded() {
    let test_db = ghost_db::pool::connect_test().await.unwrap();
    let pool = test_db.pool();
    // Nothing sent yet, so nothing to report — and no divide by zero.
    assert_eq!(super::newsletter_open_stats(&pool, "-30 days").await, (0, 0));

    sqlx::query(ghost_db::dialect::sql("INSERT INTO emails (id, post_id, status, email_count, created_at) VALUES ('an-email', 'a-post', 'submitted', 3, datetime('now'))"))
        .execute(&pool).await.unwrap();
    let now = ghost_core::dates::now_db();
    for (id, status, opened) in [("one", "sent", true), ("two", "sent", false), ("three", "failed", false)] {
        sqlx::query("INSERT INTO email_recipients (id, email_id, member_id, status, opened_at, created_at) VALUES (?, 'an-email', ?, ?, ?, ?)")
            .bind(id).bind(id).bind(status).bind(opened.then(|| now.clone())).bind(&now)
            .execute(&pool).await.unwrap();
    }
    // Two delivered, one of them opened. The message that never left is
    // not in the denominator.
    // The cutoff is an absolute instant rather than a SQLite modifier.
    let days_ago = |n: i64| {
        ghost_core::dates::to_db_string(&(chrono::Utc::now() - chrono::Duration::days(n)))
    };
    assert_eq!(super::newsletter_open_stats(&pool, &days_ago(30)).await, (2, 1));

    // An opened send from before the window does not raise the rate in it.
    sqlx::query(ghost_db::dialect::sql("INSERT INTO email_recipients (id, email_id, member_id, status, opened_at, created_at) VALUES ('old', 'an-email', 'old', 'sent', datetime('now','-60 days'), datetime('now','-60 days'))"))
        .execute(&pool).await.unwrap();
    assert_eq!(super::newsletter_open_stats(&pool, &days_ago(30)).await, (2, 1));
    assert_eq!(super::newsletter_open_stats(&pool, &days_ago(90)).await, (3, 2));
}

#[test]
fn module_pages_share_one_admin_only_sidebar_entry() {
    for active in ["modules", "analytics", "comments", "automations", "forms"] {
        let nav = super::rail(active, None, super::Role::Administrator);
        assert!(nav.contains(r#"class="nav-item active" href="/ghost/modules/""#));
        for href in ["analytics", "comments", "automations", "forms"] {
            assert!(!nav.contains(&format!(r#"class="nav-item" href="/ghost/{href}/""#)));
            assert!(!nav.contains(&format!(r#"class="nav-item active" href="/ghost/{href}/""#)));
        }
    }
    for role in [super::Role::Contributor, super::Role::Author, super::Role::Editor] {
        assert!(!super::rail("posts", None, role).contains("/ghost/modules/"));
    }
}

/// The stylesheet is linked, not inlined, and the URL the page links is
/// exactly the one `admin_asset` answers.
///
/// The hash in the query string is the whole point of the arrangement: the
/// response below is cached `immutable` for a year, so without it an edit
/// to `admin.css` would never reach a browser that had already loaded the
/// admin once.
#[tokio::test]
async fn the_stylesheet_is_linked_at_a_hashed_url_and_served_there() {
    use axum::http::header;

    let settings = std::sync::Arc::new(ghost_db::SettingsCache::from_values([("name".into(), serde_json::json!("A site"))]));
    let html = super::ADMIN_SETTINGS
        .scope(settings, async { super::page_u("Posts", "posts", None, "", true).0 })
        .await;

    let href = format!("{}/admin.css?v={}", super::ASSETS, super::admin_css_hash());
    assert!(html.contains(&format!(r#"<link rel="stylesheet" href="{href}">"#)), "{href} not linked");
    // 130 KB of rules per page response is what moving it out of here saved
    assert!(!html.contains("<style>"), "the stylesheet is inlined into the page again");
    assert!(!html.contains("--on-primary-container"));

    let hash = super::admin_css_hash();
    assert_eq!(hash.len(), 12);
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()), "{hash}");

    let response = super::admin_asset(axum::extract::Path((super::KOENIG_VERSION.to_string(), "admin.css".to_string()))).await;
    assert_eq!(response.status(), super::StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "text/css; charset=utf-8");
    assert_eq!(response.headers()[header::CACHE_CONTROL], "public, max-age=31536000, immutable");
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(std::str::from_utf8(&body).unwrap(), super::CSS);
}

/// The account block in the rail is `<span class="avatar">A</span>` next to a
/// `<div>` of name and address. Both rules that style that text were written
/// as descendant selectors — `.who span`, `.who-link span` — and a span is
/// what the avatar is, so they reached into the disc and overrode it: the
/// initial lost the grid that centres it and went to the top-left corner, at
/// the muted text colour, which on the primary disc is all but invisible.
#[test]
fn the_avatar_initial_is_not_styled_as_a_line_of_text() {
    for rule in [".who span", ".who-link b, .who-link span"] {
        assert!(
            !super::CSS.contains(&format!("\n{rule} {{")),
            "{rule} styles the avatar as text; exclude it with :not(.avatar)"
        );
    }
    assert!(super::CSS.contains(".who span:not(.avatar) {"));
    assert!(super::CSS.contains(".who-link b, .who-link span:not(.avatar) {"));
    // And nothing else has quietly taken over the centring.
    assert!(
        super::CSS.contains(".avatar { width: 30px; height: 30px;"),
        "the avatar's own rule moved; check it still centres its initial"
    );
}

#[tokio::test]
async fn admin_branding_uses_the_current_site_name_and_escapes_html() {
    let settings = std::sync::Arc::new(ghost_db::SettingsCache::from_values([
        ("title".into(), serde_json::json!("A different title")),
        ("name".into(), serde_json::json!("News <&>")),
    ]));
    super::ADMIN_SETTINGS.scope(settings, async {
        let html = super::page_u("Posts", "posts", None, "", true).0;
        assert!(html.contains("<b>News &lt;&amp;&gt;</b>"));
        assert!(html.contains("<title>Posts — News &lt;&amp;&gt;</title>"));
        assert!(!html.contains("<b>Admin</b>"));
        assert!(!html.contains("<span>Ghost</span>"));
    }).await;
}

/// The members view's NQL has to survive whatever is typed into the
/// search box: every NQL metacharacter is stripped, and the result has
/// to still parse — a member called O'Brien (VIP) is searched for by
/// typing exactly that.
#[test]
fn members_nql_strips_syntax_and_parses() {
    let nql = super::members_nql("free", "O'Brien (VIP)", "").unwrap();
    assert!(ghost_nql::parse(&nql).is_ok(), "{nql}");
    assert!(nql.contains("status:free"), "{nql}");
    assert!(!nql.contains('\'') || nql.matches('\'').count() == 6, "{nql}");

    let both = super::members_nql("paid", "ak", "").unwrap();
    assert!(ghost_nql::parse(&both).is_ok(), "{both}");
    assert!(both.starts_with("status:paid+("), "{both}");

    assert_eq!(super::members_nql("all", "", ""), None);
    assert_eq!(super::members_nql("all", "   ", ""), None);
    // an unknown status key is not a filter this page offers
    assert_eq!(super::members_nql("bogus", "", ""), None);
}

/// The toolbar's filter joins the tab and the search rather than
/// replacing them, and is parenthesised on the way in — an `or` inside a
/// typed filter must not reach across the `+` and swallow `status:`.
#[test]
fn a_typed_filter_narrows_the_tab_rather_than_replacing_it() {
    let both = super::members_nql("paid", "", "label:vip,label:beta").unwrap();
    assert_eq!(both, "status:paid+(label:vip,label:beta)");
    assert!(ghost_nql::parse(&both).is_ok(), "{both}");

    let all_three = super::members_nql("free", "ak", "label:vip").unwrap();
    assert!(ghost_nql::parse(&all_three).is_ok(), "{all_three}");
    assert!(all_three.starts_with("status:free+(label:vip)+("), "{all_three}");

    // on its own it is still the whole filter
    assert_eq!(super::members_nql("all", "", "label:vip"), Some("(label:vip)".into()));
    assert_eq!(super::members_nql("all", "", "   "), None);
}

// The Connect handshake these two screens drive — the OAuth URL, the
// session-bound state and the secure key's shape — now lives in
// `ghost_api::stripe_connect`, shared with the Admin API endpoint, and is
// tested there against the same Ghost 6.73 values.

#[test]
fn a_recommended_url_gets_a_scheme_but_must_be_a_real_host() {
    assert_eq!(recommendation_url("example.com").as_deref(), Some("https://example.com"));
    assert_eq!(
        recommendation_url(" http://example.com/blog ").as_deref(),
        Some("http://example.com/blog")
    );
    // a host with no dot is a typo here, not a publication
    assert!(recommendation_url("localhost").is_none());
    assert!(recommendation_url("").is_none());
}

#[test]
fn a_blank_title_falls_back_to_the_domain() {
    assert_eq!(recommendation_title("", "https://www.example.com/blog"), "example.com");
    assert_eq!(recommendation_title("  ", "https://example.com"), "example.com");
    assert_eq!(recommendation_title(" Real Name ", "https://example.com"), "Real Name");
}

use super::*;

#[test]
fn editor_form_collects_repeated_tag_fields() {
    // what the browser posts with three boxes ticked
    let form = EditorForm::parse(
        "title=Hello&slug=hello&markdown=Body&status=published\
         &tags=aaa111&tags=bbb222&tags=ccc333",
    );
    assert_eq!(form.tags, vec!["aaa111", "bbb222", "ccc333"]);
    assert_eq!(form.title, "Hello");
    assert_eq!(form.slug.as_deref(), Some("hello"));
}

#[test]
fn editor_form_accepts_one_tag_or_none() {
    // a single ticked box is what used to fail to deserialize
    let one = EditorForm::parse("title=T&markdown=&status=draft&tags=6ad3d43d22c4d20da6000002");
    assert_eq!(one.tags, vec!["6ad3d43d22c4d20da6000002"]);

    let none = EditorForm::parse("title=T&markdown=&status=draft");
    assert!(none.tags.is_empty());
}

#[test]
fn editor_form_decodes_and_blanks_empty_optionals() {
    let form = EditorForm::parse(
        "title=Caf%C3%A9+%26+Bar&slug=&markdown=A+%2B+B&status=draft\
         &feature_image=&custom_excerpt=++&featured=1",
    );
    assert_eq!(form.title, "Café & Bar");
    assert_eq!(form.markdown.as_deref(), Some("A + B"));
    // an empty or whitespace-only field is absent, not an empty string
    assert_eq!(form.slug, None);
    assert_eq!(form.feature_image, None);
    assert_eq!(form.custom_excerpt, None);
    // a ticked checkbox posts at all; an unticked one posts nothing
    assert!(form.featured);
    assert!(!form.show_title_and_feature_image);
}

#[test]
fn koenig_html_is_preferred_over_rendering_the_lexical() {
    let lexical = r#"{"root":{"children":[{"type":"paragraph","version":1,"children":[{"text":"hi","type":"text","format":0,"version":1}]}],"type":"root","version":1}}"#;
    let body = format!(
        "title=T&status=draft&lexical={}&html={}",
        form_urlencoded::byte_serialize(lexical.as_bytes()).collect::<String>(),
        form_urlencoded::byte_serialize(b"<p>from the browser</p>").collect::<String>(),
    );
    let form = EditorForm::parse(&body);
    assert!(form.is_koenig());
    let (html, plaintext, _) = form.rendered();
    assert_eq!(html, "<p>from the browser</p>");
    assert_eq!(plaintext, "from the browser");
}

#[test]
fn lexical_without_browser_html_is_rendered_server_side() {
    let lexical = r#"{"root":{"children":[{"type":"paragraph","version":1,"children":[{"text":"hi","type":"text","format":0,"version":1}]}],"type":"root","version":1}}"#;
    let body = format!(
        "title=T&status=draft&lexical={}",
        form_urlencoded::byte_serialize(lexical.as_bytes()).collect::<String>()
    );
    let (html, plaintext, reading_time) = EditorForm::parse(&body).rendered();
    assert_eq!(html, "<p>hi</p>");
    assert_eq!(plaintext, "hi");
    assert_eq!(reading_time, 1);
}

#[test]
fn markdown_fallback_still_renders_when_koenig_did_not_run() {
    let form = EditorForm::parse("title=T&status=draft&markdown=%23+Heading");
    assert!(!form.is_koenig());
    let (html, _, _) = form.rendered();
    assert!(html.contains("<h1>Heading</h1>"), "{html}");
}

#[test]
fn each_editor_may_only_clear_the_other_source_it_replaces() {
    // A Koenig save blanks any leftover markdown, and a Markdown save
    // blanks any leftover lexical — never the one it did not write.
    assert!(editor_owned_fields(true, true).contains(&"markdown"));
    assert!(!editor_owned_fields(true, true).contains(&"lexical"));
    assert!(editor_owned_fields(false, true).contains(&"lexical"));
    assert!(!editor_owned_fields(false, true).contains(&"markdown"));
}

#[test]
fn a_save_carrying_no_body_leaves_the_content_alone() {
    // Every settings field posts, but neither editor did — the content
    // columns must not be cleared or re-rendered from nothing.
    let form = EditorForm::parse("title=T&status=draft&slug=t&meta_title=M");
    assert!(!form.has_content());
    for field in ["html", "plaintext", "lexical", "markdown"] {
        assert!(!editor_owned_fields(form.is_koenig(), form.has_content()).contains(&field), "{field} would be cleared");
    }
    // the settings on the same request are still applied
    assert!(editor_owned_fields(false, false).contains(&"meta_title"));
}

#[test]
fn publish_dates_round_trip_through_the_datetime_input() {
    assert_eq!(to_datetime_local("2026-10-19 14:30:00"), "2026-10-19T14:30");
    assert_eq!(from_datetime_local("2026-10-19T14:30").as_deref(), Some("2026-10-19 14:30:00"));
    // an empty or half-filled input leaves the date alone
    assert_eq!(to_datetime_local(""), "");
    assert_eq!(from_datetime_local(""), None);
    assert_eq!(from_datetime_local("2026-10-19"), None);
}

#[test]
fn editor_form_ignores_unknown_fields() {
    let form = EditorForm::parse("title=T&markdown=M&status=draft&csrf=xyz&_method=put");
    assert_eq!(form.title, "T");
    assert_eq!(form.status, "draft");
}

#[test]
fn nav_form_pairs_labels_with_urls_in_row_order() {
    // two primary rows and one secondary row, as the browser posts them
    let form = NavForm::parse(
        "primary_label=Home&primary_url=%2F&primary_label=About&primary_url=%2Fabout%2F\
         &secondary_label=Sign+up&secondary_url=%23%2Fportal%2F&action=save",
    );
    assert_eq!(
        form.primary,
        vec![NavItem { label: "Home".into(), url: "/".into() }, NavItem { label: "About".into(), url: "/about/".into() }]
    );
    assert_eq!(
        form.secondary,
        vec![NavItem { label: "Sign up".into(), url: "#/portal/".into() }]
    );
    assert_eq!(form.action, "save");
}

#[test]
fn nav_form_pads_uneven_label_url_counts() {
    // a row with a label but no matching url still shows up, url blank
    let form = NavForm::parse("primary_label=Home&primary_url=%2F&primary_label=Blog");
    assert_eq!(
        form.primary,
        vec![NavItem { label: "Home".into(), url: "/".into() }, NavItem { label: "Blog".into(), url: String::new() }]
    );
}

#[test]
fn nav_row_actions_move_add_and_remove() {
    let mut form = NavForm {
        primary: vec![
            NavItem { label: "A".into(), url: "/a/".into() },
            NavItem { label: "B".into(), url: "/b/".into() },
            NavItem { label: "C".into(), url: "/c/".into() },
        ],
        ..Default::default()
    };

    // down moves B below C
    assert!(nav_apply_action(&mut form, "down-primary-1"));
    assert_eq!(form.primary.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), ["A", "C", "B"]);
    // up on the first row is a no-op
    assert!(nav_apply_action(&mut form, "up-primary-0"));
    assert_eq!(form.primary.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), ["A", "C", "B"]);
    // remove takes out the row at the index
    assert!(nav_apply_action(&mut form, "remove-primary-0"));
    assert_eq!(form.primary.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), ["C", "B"]);
    // add appends an empty row; save is not a row action
    assert!(nav_apply_action(&mut form, "add-primary"));
    assert_eq!(form.primary.last(), Some(&NavItem::default()));
    assert!(!nav_apply_action(&mut form, "save"));
    // out-of-range and unknown actions re-render without mutating
    assert!(nav_apply_action(&mut form, "remove-primary-99"));
    assert!(nav_apply_action(&mut form, "teleport-primary-0"));
    assert_eq!(form.primary.len(), 3);
}

#[test]
fn publish_actions_map_to_status_and_sending() {
    use PublishAction::*;
    // "Publish only" leaves the requested status alone and mails nobody
    assert_eq!(PublishAction::parse("publish_only"), PublishOnly);
    assert!(!PublishOnly.sends_email());
    assert_eq!(PublishOnly.status_for("published"), "published");

    // "Publish and email" publishes *and* sends
    assert_eq!(PublishAction::parse("publish_email"), PublishAndEmail);
    assert!(PublishAndEmail.sends_email());
    assert_eq!(PublishAndEmail.status_for("published"), "published");

    // "Email only" overrides the status to `sent`, so the post goes to
    // subscribers without appearing in the site's listings
    assert_eq!(PublishAction::parse("email_only"), EmailOnly);
    assert!(EmailOnly.sends_email());
    assert_eq!(EmailOnly.status_for("published"), "sent");
    assert_eq!(EmailOnly.status_for("draft"), "sent");

    // anything unrecognised must not send — failing closed matters when
    // the alternative is mailing every subscriber by accident
    assert_eq!(PublishAction::parse(""), PublishOnly);
    assert_eq!(PublishAction::parse("nonsense"), PublishOnly);
}

/// A scheduled save is the one case where the decision is stored instead
/// of acted on, so the storing has to be right: a bad date must never
/// become "now", and an email intent must survive to the delivery row.
#[test]
fn scheduling_reads_the_date_and_the_delivery() {
    // The comparison is textual, so fixed dates either side of any
    // plausible "now" make this independent of the clock.
    const FUTURE: &str = "2099-01-01 09:00:00";
    const PAST: &str = "2000-01-01 09:00:00";
    let form = |status: &str, when: &str, action: &str| {
        let mut f = EditorForm::parse("title=T&markdown=body");
        f.status = status.into();
        f.published_at = (!when.is_empty()).then(|| when.to_string());
        f.publish_action = action.into();
        f
    };

    // not scheduled: nothing to arrange, whatever the date says
    let f = form("published", FUTURE, "publish_email");
    assert!(matches!(scheduling(&f, "post", PublishAction::PublishAndEmail), Scheduling::No));

    // scheduled into the future, with and without an email
    let f = form("scheduled", FUTURE, "publish_only");
    let Scheduling::At(d) = scheduling(&f, "post", PublishAction::PublishOnly) else {
        panic!("a future date is a schedule");
    };
    assert!(!d.email && !d.email_only);
    assert_eq!(d.status(), "published");

    let f = form("scheduled", FUTURE, "publish_email");
    let Scheduling::At(d) = scheduling(&f, "post", PublishAction::PublishAndEmail) else {
        panic!("a future date is a schedule");
    };
    assert!(d.email && !d.email_only);
    assert_eq!(d.status(), "published");

    let f = form("scheduled", FUTURE, "email_only");
    let Scheduling::At(d) = scheduling(&f, "post", PublishAction::EmailOnly) else {
        panic!("a future date is a schedule");
    };
    assert!(d.email && d.email_only);
    // email-only posts land in Ghost's `sent`, off the site's listings
    assert_eq!(d.status(), "sent");

    // a page has no newsletter, so the choice cannot make it send one
    let f = form("scheduled", FUTURE, "publish_email");
    let Scheduling::At(d) = scheduling(&f, "page", PublishAction::PublishAndEmail) else {
        panic!("a page can be scheduled too");
    };
    assert!(!d.email);

    // a date in the past, or none at all, is not a schedule — and must
    // not fall through to publishing immediately
    for when in [PAST, ""] {
        let f = form("scheduled", &when, "publish_email");
        assert!(
            matches!(scheduling(&f, "post", PublishAction::PublishAndEmail), Scheduling::Undated),
            "{when:?} was accepted as a schedule"
        );
    }
}

#[test]
fn editor_form_defaults_to_not_sending() {
    // a plain draft save posts no publish_action at all
    let f = EditorForm::parse("title=T&markdown=body&status=draft");
    assert_eq!(PublishAction::parse(&f.publish_action), PublishAction::PublishOnly);
}

#[test]
fn roles_match_ghosts_permission_fixtures() {
    use Role::*;
    // Ghost grants no `member`, `integration`, `api_key` or `db`
    // permission below Administrator, and `setting` is browse/read only
    // for Editor and under — so settings, members, themes and
    // integrations are admin territory.
    for r in [Contributor, Author, Editor] {
        assert!(!r.is_admin(), "{r:?} must not reach admin-only screens");
    }
    for r in [Administrator, Owner] {
        assert!(r.is_admin());
    }

    // tag = all for Editor and above; Authors may only add while writing
    assert!(!Contributor.can_manage_tags());
    assert!(!Author.can_manage_tags());
    assert!(Editor.can_manage_tags());

    // post = all for Editor; Authors and Contributors are scoped to their own
    assert!(!Contributor.can_edit_any_post());
    assert!(!Author.can_edit_any_post());
    assert!(Editor.can_edit_any_post());

    // publishing is the line Ghost draws for Contributors
    assert!(!Contributor.can_publish());
    assert!(Author.can_publish());

    // Editors manage Authors and Contributors
    assert!(!Author.can_manage_staff());
    assert!(Editor.can_manage_staff());
}

#[test]
fn an_unknown_role_gets_the_least_privilege() {
    // a renamed or missing role must never be read as trusted
    assert_eq!(Role::from_name(""), Role::Contributor);
    assert_eq!(Role::from_name("Nonsense"), Role::Contributor);
    assert_eq!(Role::from_name("Admin Integration"), Role::Contributor);
    // and the real names still map
    assert_eq!(Role::from_name("Owner"), Role::Owner);
    assert_eq!(Role::from_name("Administrator"), Role::Administrator);
    assert_eq!(Role::from_name("Editor"), Role::Editor);
    assert_eq!(Role::from_name("Author"), Role::Author);
    assert_eq!(Role::from_name("Contributor"), Role::Contributor);
}

#[test]
fn roles_are_ordered_by_privilege() {
    use Role::*;
    assert!(Owner > Administrator && Administrator > Editor && Editor > Author && Author > Contributor);
}

#[test]
fn member_csv_columns_match_ghosts() {
    // Ghost's @tryghost/members-csv DEFAULT_COLUMNS, in order. A file
    // exported here has to import into Ghost unchanged, so drift in this
    // list silently breaks interoperability.
    assert_eq!(
        MEMBER_CSV_COLUMNS,
        [
            "id", "email", "name", "note", "subscribed_to_emails", "complimentary_plan",
            "stripe_customer_id", "created_at", "deleted_at", "labels", "tiers",
        ]
    );
    // email is what rows are matched on, so it must always be present
    assert!(MEMBER_CSV_COLUMNS.contains(&"email"));
}

#[test]
fn member_edit_form_collects_repeated_newsletter_boxes() {
    // every ticked newsletter posts under the same name, which a typed
    // Form extractor would collapse to the last one
    let f = MemberEditForm::parse(
        "name=Ada&email=ada%40example.com&note=VIP&labels=a%2C+b&status=comped\
         &newsletters=n1&newsletters=n2&newsletters=n3",
    );
    assert_eq!(f.newsletters, vec!["n1", "n2", "n3"]);
    assert_eq!(f.email, "ada@example.com");
    assert_eq!(f.status, "comped");
    assert_eq!(f.labels, "a, b");
}

#[test]
fn member_edit_form_with_no_newsletters_means_unsubscribed() {
    // the absence of the field is the signal, so it must parse as empty
    // rather than being treated as "unchanged"
    let f = MemberEditForm::parse("name=Ada&email=ada%40example.com&status=free");
    assert!(f.newsletters.is_empty());
}

#[test]
fn nav_saves_when_no_action_was_posted() {
    // A submit button contributes its name/value only when it is the
    // element that activated the form, so the toolbar Save button and
    // Enter-in-a-field both post with no `action` at all. Treating that
    // as an unknown row action silently discarded the whole edit.
    let mut form = NavForm {
        primary: vec![NavItem { label: "Home".into(), url: "/".into() }],
        ..Default::default()
    };
    assert!(!nav_apply_action(&mut form, ""), "an absent action must save, not re-render");
    assert_eq!(form.primary.len(), 1, "and must not mutate the rows on the way");
}

#[test]
fn nav_save_button_posts_an_explicit_action() {
    // belt and braces for the above: the rendered toolbar button carries
    // the field, so the save path does not depend on the empty-string
    // fallback alone
    let html = navigation_html(&[], &[], "");
    assert!(
        html.contains(r#"type="submit" form="nav-form" name="action" value="save""#),
        "toolbar Save must submit action=save"
    );
}

#[test]
fn nav_round_trips_a_posted_form() {
    // the exact body the browser sends when Save is clicked
    let form = NavForm::parse(
        "primary_label=Home&primary_url=%2F\
         &primary_label=About&primary_url=%2Fabout%2F\
         &secondary_label=Privacy&secondary_url=%2Fprivacy%2F&action=save",
    );
    assert_eq!(form.action, "save");
    let primary = nav_clean(&form.primary);
    assert_eq!(primary.len(), 2);
    assert_eq!(primary[1]["label"], serde_json::json!("About"));
    assert_eq!(primary[1]["url"], serde_json::json!("/about/"));
    assert_eq!(nav_clean(&form.secondary).len(), 1);
}

#[test]
fn nav_clean_drops_blank_rows_and_defaults_url() {
    let items = vec![
        NavItem { label: "Home".into(), url: "".into() },
        NavItem { label: "  ".into(), url: "/gone/".into() }, // blank label → deleted
        NavItem { label: " About ".into(), url: " /about/ ".into() },
    ];
    assert_eq!(
        nav_clean(&items),
        vec![
            serde_json::json!({"label": "Home", "url": "/"}),
            serde_json::json!({"label": "About", "url": "/about/"})
        ]
    );
}

#[test]
fn nav_items_read_back_from_array_or_json_string() {
    // the settings cache stores arrays; the Admin API may store the same
    // value as a JSON-encoded string
    let arr = nav_items_from_setting(Some(serde_json::json!([{"label": "Home", "url": "/"}])));
    let s = nav_items_from_setting(Some(serde_json::json!("[{\"label\":\"Home\",\"url\":\"/\"}]")));
    assert_eq!(arr, vec![NavItem { label: "Home".into(), url: "/".into() }]);
    assert_eq!(arr, s);
    assert!(nav_items_from_setting(None).is_empty());
}

#[test]
fn money_minor_groups_and_drops_whole_cents() {
    // the subscriptions.mrr column is in minor units, like Stripe's
    assert_eq!(money_minor(0), "0");
    assert_eq!(money_minor(500), "5");
    assert_eq!(money_minor(1250), "12.50");
    assert_eq!(money_minor(123456), "1,234.56");
    assert_eq!(money_minor(100000000), "1,000,000");
    assert_eq!(money_minor(-2550), "-25.50");
}

#[test]
fn countable_agrees_with_its_noun() {
    assert_eq!(countable(1, "visitor"), "1 visitor");
    assert_eq!(countable(0, "visitor"), "0 visitors");
    assert_eq!(countable(2, "view"), "2 views");
    assert_eq!(countable(2500, "member"), "2.5K members");
}

#[test]
fn audience_clause_only_ever_emits_known_segments() {
    // the value reaches here from the query string, so it must never be
    // interpolated — it is matched against a fixed set instead
    assert_eq!(audience_clause("paid"), " AND audience = 'paid'");
    assert_eq!(audience_clause("all"), "");
    assert_eq!(audience_clause("'; DROP TABLE page_views;--"), "");
}

#[test]
fn analytics_range_falls_back_to_thirty_days() {
    let any = |v: &str| {
        let mut q = HashMap::new();
        q.insert("range".to_string(), v.to_string());
        AnalyticsRange::from_query(&q)
    };
    // The cutoff is an absolute instant now, not a SQLite modifier, so
    // what matters is which instant it names.
    let seven = ghost_core::dates::from_db_string(&any("7").cutoff)
        .expect("the cutoff is a database timestamp");
    let back = (chrono::Utc::now() - seven).num_days();
    assert_eq!(back, 7, "a 7-day range should start 7 days ago, not {back}");

    // "Today" starts at midnight, which is what `start of day` meant.
    let today = any("today").cutoff;
    assert!(today.ends_with(" 00:00:00"), "{today}");
    assert!(today.starts_with(&chrono::Utc::now().format("%Y-%m-%d").to_string()), "{today}");

    // All time has to reach back past any real content.
    assert!(any("all").cutoff < "2000-01-01 00:00:00".to_string(), "{}", any("all").cutoff);
    assert_eq!(any("all").days, None);
    // an unknown range is the default rather than an error page
    assert_eq!(any("bogus").slug, "30");
    assert_eq!(AnalyticsRange::from_query(&HashMap::new()).slug, "30");
}

#[test]
fn settings_groups_match_ghosts_navigation() {
    // Ghost's settings sidebar, in order. Email and Bulk email sit after
    // Membership: Ghost keeps both in config.json, this port exposes them
    // in the UI, and they belong next to the membership features that
    // send the mail. Features is this port's own group — the Comments
    // and Automations system switches have no Ghost page to mirror —
    // and so is Machine payments, which slots in beside Membership as
    // the other thing readers (and agents) can be charged for.
    //
    // Design & branding and Navigation moved here out of the left rail,
    // which is also where Ghost keeps them; Announcement is what is left
    // of the old Site group once its Theme row — a second door onto
    // Design — was taken away. Integrations and Code injection are
    // Ghost's own settings pages too; they lived under Advanced here
    // until each grew a Save button of its own.
    //
    // Meta data and Social accounts are no longer groups: they are
    // sections of General settings, beside the title and description
    // every one of their fields falls back to. Migration tools is a group
    // in its place, holding the import and export that were under
    // Advanced — which is Ghost's own arrangement.
    let titles: Vec<&str> = SETTINGS_GROUPS.iter().map(|g| g.title).collect();
    assert_eq!(
        titles,
        ["General settings", "Staff", "Analytics",
         "Design & branding", "Navigation", "Code injection", "Announcement",
         "Membership", "Machine payments", "Email", "Newsletters", "Growth",
         "Features", "Forms", "Integrations",
         "Migration tools", "Advanced", "History", "Danger zone"]
    );
    // every group has a working page at its slug
    assert!(SETTINGS_GROUPS.iter().all(|g| !g.slug.is_empty() && !g.items.is_empty()));
    // and an icon the set actually holds — an unknown name renders an
    // empty square where every other row has a glyph
    assert!(SETTINGS_GROUPS.iter().all(|g| !icon(g.icon).is_empty()), "missing icon");
    assert!(MODULES.iter().all(|m| !icon(m.2).is_empty()), "missing icon");
    // the two groups that keep their own top-level page are the only ones
    // that do not live under /ghost/settings/
    for g in SETTINGS_GROUPS {
        let expected_top_level = matches!(g.slug, "design" | "navigation");
        assert_eq!(
            !g.path().starts_with("/ghost/settings/"),
            expected_top_level,
            "{} points at {}",
            g.title,
            g.path()
        );
    }
}

#[test]
fn every_searchable_setting_lands_on_a_real_page() {
    // a typo'd slug in the index would render nowhere — the row is
    // silently dropped — so the index is checked against the groups
    for (slug, section, field) in SETTINGS_INDEX {
        assert!(
            SETTINGS_GROUPS.iter().any(|g| g.slug == *slug),
            "SETTINGS_INDEX points at unknown settings page: {slug}"
        );
        assert!(!section.is_empty() && !field.is_empty(), "empty section or field for {slug}");
    }
    for (_, name, parent) in SETTINGS_PLACES {
        assert!(
            SETTINGS_GROUPS.iter().any(|g| g.slug == *parent),
            "SETTINGS_PLACES points at unknown settings page: {parent} ({name})"
        );
    }
}

#[test]
fn announcement_visibility_reads_array_or_json_string() {
    let arr = announcement_visibility_set(Some(serde_json::json!(["visitors", "paid_members"])));
    assert_eq!(arr, vec!["visitors", "paid_members"]);
    let s = announcement_visibility_set(Some(serde_json::json!("[\"free_members\"]")));
    assert_eq!(s, vec!["free_members"]);
    assert!(announcement_visibility_set(None).is_empty());
    assert!(announcement_visibility_set(Some(serde_json::json!("not json"))).is_empty());
}

#[test]
fn ghost_export_has_ghosts_shape() {
    let post = ghost_core::models::Post {
        id: "p1".into(),
        title: Some("Hello".into()),
        slug: Some("hello".into()),
        markdown: Some("Body".into()),
        post_type: "post".into(),
        status: "published".into(),
        ..Default::default()
    };
    let tag = ghost_core::models::Tag {
        id: "t1".into(),
        name: Some("News".into()),
        ..Default::default()
    };
    let export = build_ghost_export(std::slice::from_ref(&post), std::slice::from_ref(&tag), "http://x/");
    let entry = &export["db"][0];
    assert!(entry["meta"]["exported_on"].is_u64());
    assert_eq!(entry["data"]["posts"][0]["slug"], "hello");
    assert_eq!(entry["data"]["tags"][0]["name"], "News");
}

#[test]
fn import_counts_message_pluralises() {
    let one = ImportCounts { posts: 1, pages: 1, tags: 1 };
    assert_eq!(one.message(), "Imported 1 post, 1 tag and 1 page.");
    let many = ImportCounts { posts: 3, pages: 2, tags: 4 };
    assert_eq!(many.message(), "Imported 3 posts, 4 tags and 2 pages.");
}

#[test]
fn tier_prices_collapse_whole_dollars() {
    assert_eq!(tier_price(None), "—");
    assert_eq!(tier_price(Some(500)), "$5");
    assert_eq!(tier_price(Some(499)), "$4.99");
}

#[test]
fn pickers_contain_the_seeded_defaults() {
    // a fresh site stores Etc/UTC / en — both must be selectable
    assert!(TIMEZONES.iter().any(|(v, _)| *v == "Etc/UTC"));
    assert!(LOCALES.iter().any(|(v, _)| *v == "en"));
}

#[test]
fn select_options_mark_current_and_admit_unknown_values() {
    let list = [("a", "Alpha"), ("b", "Beta")];
    let opts = select_options(&list, "a");
    assert!(opts.contains(r#"<option value="a" selected>Alpha</option>"#));
    assert!(!opts.contains("selected>Beta"));

    // a stored value outside the list still shows up as a raw option
    let opts = select_options(&list, "zz");
    assert!(opts.starts_with(r#"<option value="zz" selected>zz</option>"#));
    assert!(opts.contains(r#"<option value="a">Alpha</option>"#));
}

#[test]
fn compact_numbers_and_clean_axes() {
    assert_eq!(compact(0), "0");
    assert_eq!(compact(999), "999");
    assert_eq!(compact(1284), "1.3K");
    assert_eq!(compact(2_400_000), "2.4M");
    assert_eq!(clean_max(1), 4);
    assert_eq!(clean_max(6), 8);
    assert_eq!(clean_max(70), 80);
    assert_eq!(clean_max(100), 100);
    assert_eq!(clean_max(1234), 2000);
}

#[test]
fn views_chart_draws_bars_grid_and_a_max_label() {
    let days = vec![
        DayViews { day: "2026-10-16".into(), views: 4 },
        DayViews { day: "2026-10-17".into(), views: 0 },
        DayViews { day: "2026-10-18".into(), views: 12 },
    ];
    let svg = views_chart(&days);
    assert!(svg.contains("aria-label=\"Views per day\""));
    assert!(svg.contains("2026-10-18: 12 views"), "every bar carries a tooltip");
    assert!(svg.contains(">12</text>"), "the max value is labeled");
    assert_eq!(svg.matches("class=\"bar\"").count(), 2, "zero days draw no bar");
    assert!(svg.contains(">10-17</text>"), "day labels are MM-DD");

    // empty window renders a note, not a broken chart
    assert!(views_chart(&[]).contains("No views recorded yet"));
}

#[test]
fn member_uuids_are_v4_shaped() {
    let id = ghost_core::ids::uuid_v4();
    assert_eq!(id.len(), 36);
    assert_eq!(id.chars().filter(|c| *c == '-').count(), 4);
    assert_eq!(&id[14..15], "4", "version nibble");
    assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"), "variant nibble");
}

#[test]
fn custom_values_coerce_by_declared_type() {
    // a theme declaring one field of each supported type
    let decls: serde_json::Map<String, serde_json::Value> = serde_json::from_value(serde_json::json!({
        "site_background_color": {"type": "color", "default": "#ffffff"},
        "sticky_header": {"type": "boolean", "default": true},
        "navigation_layout": {"type": "select", "options": ["A", "B"], "default": "A"},
        "hero_title": {"type": "text", "default": "Hi"}
    }))
    .unwrap();

    // booleans absent from the post (unticked) and selects off-list fall
    // back; colors/texts take the posted strings
    let mut form = HashMap::new();
    form.insert("site_background_color".into(), "#123456".into());
    form.insert("navigation_layout".into(), "NOT-A-CHOICE".into());
    form.insert("hero_title".into(), "Hello".into());
    let values = coerce_custom_values(&decls, &form);
    assert_eq!(values["site_background_color"], serde_json::json!("#123456"));
    assert_eq!(values["sticky_header"], serde_json::json!(false));
    assert_eq!(values["navigation_layout"], serde_json::json!("A"));
    assert_eq!(values["hero_title"], serde_json::json!("Hello"));
    assert!(!values.contains_key("not_declared"));

    // a ticked boolean posts "1" alongside its hidden companion marker
    let mut form = HashMap::new();
    form.insert("_sticky_header".into(), "1".into());
    form.insert("sticky_header".into(), "1".into());
    let values = coerce_custom_values(&decls, &form);
    assert_eq!(values["sticky_header"], serde_json::json!(true));
}

#[test]
fn portal_links_carry_both_spellings_of_each_destination() {
    let tiers = vec![("tier1".to_string(), "Gold".to_string())];
    let html = portal_links_card("https://site.test/", &tiers, true, true);

    // The link and the attribute ride on the same row, because the switch
    // between them happens in the browser with no round trip.
    assert!(html.contains(r#"data-link="https://site.test/#/portal/signup""#));
    assert!(html.contains(r#"data-attr="data-portal=&quot;signup&quot;""#));
    // The bare default has no value on the attribute at all.
    assert!(html.contains(r#"data-link="https://site.test/#/portal" data-attr="data-portal""#));
    // A trailing slash on the site URL must not double up.
    assert!(!html.contains("site.test//#"));
    // Tier rows keep the placeholder; the picker fills it client-side.
    assert!(html.contains("#/portal/signup/{tier}/monthly"));
    assert!(html.contains(r#"<option value="tier1" selected>Gold</option>"#));
}

#[test]
fn portal_links_hide_what_the_site_cannot_do() {
    // No Stripe: no paid signup, no gifts, no tips, no plan switching —
    // Portal refuses to open those, so offering the links would mislead.
    let html = portal_links_card("https://site.test", &[], false, false);
    for absent in [
        "signup/monthly",
        "gift",
        "support",
        "account/plans",
        "recommendations",
    ] {
        let row = format!(r#"data-link="https://site.test/#/portal/{absent}""#);
        assert!(!html.contains(&row), "{absent} should be hidden");
    }
    assert!(!html.contains("<h3>Tiers</h3>"));
    // What survives is what a free members site really answers.
    assert!(html.contains("#/portal/signup/free"));
    assert!(html.contains("#/portal/account/newsletters"));
}

#[test]
fn the_tier_block_needs_a_tier_to_name() {
    // `#/portal/signup//monthly` opens nothing, so with Stripe connected
    // but no paid tier priced yet the whole group stays away.
    let html = portal_links_card("https://site.test", &[], true, false);
    assert!(!html.contains("<h3>Tiers</h3>"));
    assert!(html.contains(r#"data-link="https://site.test/#/portal/gift""#));
}

#[test]
fn the_file_tree_mirrors_the_paths_it_was_built_from() {
    let files: Vec<String> = [
        "assets/css/screen.css",
        "assets/screenshot.jpg",
        "default.hbs",
        "partials/icons/arrow.hbs",
        "partials/card.hbs",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let tree = FileTree::build(&files);

    // Directories nest; files sit in the directory that owns them.
    let names: Vec<&str> = tree.dirs.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["assets", "partials"]);
    assert_eq!(tree.files, vec![("default.hbs".to_string(), "default.hbs".to_string())]);

    let assets = &tree.dirs[0].1;
    assert_eq!(assets.dirs.len(), 1, "assets holds css");
    assert_eq!(
        assets.files,
        vec![("assets/screenshot.jpg".to_string(), "screenshot.jpg".to_string())],
        "a leaf keeps its full path for the link and its bare name for the label"
    );
    assert_eq!(
        assets.dirs[0].1.files,
        vec![("assets/css/screen.css".to_string(), "screen.css".to_string())]
    );
}

#[test]
fn only_the_open_files_folders_start_expanded() {
    let files: Vec<String> =
        ["assets/css/screen.css", "partials/card.hbs"].iter().map(|s| s.to_string()).collect();
    let html = render_file_tree(&FileTree::build(&files), "casper", "assets/css/screen.css", "", 0);

    // Both directories on the path to the open file are open; the
    // unrelated one is not, or a deep assets tree buries the templates.
    assert!(html.contains(r#"<details class="fdir" open><summary style="--d:0"><span class="tw">"#));
    assert_eq!(html.matches("<details class=\"fdir\" open>").count(), 2);
    assert_eq!(html.matches("<details class=\"fdir\">").count(), 1);
    // The open file is the marked row, and links by full path.
    assert!(html.contains(r#"class="frow active""#));
    assert!(html.contains("file=assets%2Fcss%2Fscreen.css"));
}

#[test]
fn a_binary_file_is_listed_but_marked() {
    let files: Vec<String> = vec!["logo.png".to_string(), "index.hbs".to_string()];
    let html = render_file_tree(&FileTree::build(&files), "casper", "index.hbs", "", 0);
    assert!(html.contains("logo.png"), "binaries are still listed");
    assert_eq!(html.matches(r#"<span class="chip draft">binary</span>"#).count(), 1);
}

#[test]
fn the_export_bundle_offers_ghosts_components() {
    // Ghost's five synchronous components, then media, then this port's
    // extended-state component for a full backup.
    let keys: Vec<&str> = EXPORT_COMPONENTS.iter().map(|(k, _, _, _)| *k).collect();
    assert_eq!(keys, ["content", "members", "analytics", "themes", "routes", "media", "state"]);
    // Media defaults off because it is usually the bulk of the archive and
    // a content move rarely wants to wait for it. `state` defaults off
    // because it carries members, keys and sessions: the default export is
    // the one a publisher may hand to somebody else.
    let off: Vec<&str> = EXPORT_COMPONENTS
        .iter()
        .filter(|(_, _, _, on)| !on)
        .map(|(k, _, _, _)| *k)
        .collect();
    assert_eq!(off, ["media", "state"]);
}

#[test]
fn post_analytics_columns_only_promise_what_is_measured() {
    for present in ["id", "title", "url", "sends", "opens", "views", "feedback_more_like_this", "clicks", "signups", "paid_conversions"] {
        assert!(POST_ANALYTICS_COLUMNS.contains(&present), "{present} is missing");
    }
    let unique:std::collections::HashSet<_>=POST_ANALYTICS_COLUMNS.iter().collect();
    assert_eq!(unique.len(),POST_ANALYTICS_COLUMNS.len());
}

#[test]
fn media_archives_include_audio_video_images_and_files() {
    let root=std::env::temp_dir().join(format!("ghost-media-export-{}",ghost_core::ids::new_id()));
    let config: ghost_core::config::Config=serde_json::from_value(serde_json::json!({
        "url":"https://example.test","server":{},"database":{"connection":{"filename":":memory:"}},
        "paths":{"contentPath":root}
    })).unwrap();
    let entries=[("images/2026/photo.png",b"image".as_slice()),("files/2026/document.pdf",b"file".as_slice()),
        ("media/2026/clip.mp4",b"video".as_slice()),("media/2026/recording.mp3",b"audio".as_slice())];
    for (name,bytes) in entries {
        let path=config.content_path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path,bytes).unwrap();
    }
    let mut zip=zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (dir,prefix) in archive_media_paths(&config) {
        zip_add_dir(&mut zip,&dir,prefix,zip::write::SimpleFileOptions::default()).unwrap();
    }
    let mut archive=zip::ZipArchive::new(zip.finish().unwrap()).unwrap();
    assert_eq!(archive.len(),4);
    for (name,bytes) in entries {
        let archived=format!("content/{name}");
        let mut entry=archive.by_name(&archived).unwrap();
        let mut restored=Vec::new();
        std::io::Read::read_to_end(&mut entry,&mut restored).unwrap();
        assert_eq!(restored,bytes);
        assert_eq!(archive_media_target(&config,&format!("backup/{archived}")),Some(config.content_path().join(name)));
    }
    for bad in ["content/media/../../private","content/media/","mycontent/media/clip.mp4"] {
        assert!(archive_media_target(&config,bad).is_none());
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn archive_paths_cannot_escape_the_content_directory() {
    // An uploaded zip names its own entries, so this is the check that
    // keeps a crafted archive from writing outside content/.
    assert_eq!(after_segment("content/images/2026/01/a.png", "content/images/"),
               Some("2026/01/a.png".to_string()));
    // Nested one level down, as Ghost's own export is.
    assert_eq!(after_segment("backup/content/images/a.png", "content/images/"),
               Some("a.png".to_string()));
    for bad in [
        "content/images/../../../etc/passwd",
        "content/images/..",
        "content/images/a/../../b",
        "content/images/",
    ] {
        assert_eq!(after_segment(bad, "content/images/"), None, "{bad} must be refused");
    }
    // A directory that merely ends in the same letters is not a match.
    assert_eq!(after_segment("mycontent/images/a.png", "content/images/"), None);
}

#[test]
fn metadata_and_social_keys_are_all_saved_by_general() {
    // The merged page owns these; a key here that general_save does not
    // write is a field that silently resets on every save.
    assert!(METADATA_KEYS.contains(&"meta_title"));
    assert!(METADATA_KEYS.contains(&"og_image"));
    assert!(METADATA_KEYS.contains(&"twitter_description"));
    assert_eq!(METADATA_KEYS.len(), 8);
    // Social keys come from the same table the index and the form use.
    assert!(SOCIAL_ACCOUNTS.iter().any(|(k, _)| *k == "bluesky"));
}

#[test]
fn no_ga_flag_is_offered_as_a_switch() {
    // Ghost forces its GA flags true and drops them from the writable
    // allowlist, so a toggle for one would be a control that does
    // nothing — and that reads as a broken setting, not a no-op.
    let offered: Vec<&str> = BETA_FLAGS
        .iter()
        .chain(PRIVATE_FLAGS.iter())
        .map(|(k, _, _)| *k)
        .collect();
    for ga in ghost_core::LABS_GA_FLAGS {
        assert!(!offered.contains(ga), "{ga} went GA and must not be a switch");
    }
}

#[test]
fn labs_flags_match_ghost_6_73() {
    // Ghost 6.73's PUBLIC_BETA_FEATURES, in its order.
    let beta: Vec<&str> = BETA_FLAGS.iter().map(|(k, _, _)| *k).collect();
    assert_eq!(
        beta,
        ["superEditors", "editorExcerpt", "additionalPaymentMethods", "navigationIcons"]
    );
    // `automations` moved out of the beta tab and into the private one.
    assert!(PRIVATE_FLAGS.iter().any(|(k, _, _)| *k == "automations"));
    assert!(!beta.contains(&"automations"));
    // No flag is offered twice across the two tabs.
    let mut all: Vec<&str> =
        BETA_FLAGS.iter().chain(PRIVATE_FLAGS.iter()).map(|(k, _, _)| *k).collect();
    let total = all.len();
    all.sort_unstable();
    all.dedup();
    assert_eq!(all.len(), total, "a flag is listed in both tabs");
}

fn embed(layout: &str, labels: &[&str]) -> String {
    signup_form_snippet(SignupFormEmbed {
        site_url: "https://site.test",
        layout,
        background: "#ffffff",
        button: "#15171A",
        title: "My Site",
        description: "A description",
        icon: "/content/images/2026/01/icon.png",
        locale: "en",
        labels,
        preview: false,
    })
}

#[test]
fn the_signup_embed_loads_the_cdn_script_not_a_local_path() {
    // The whole point: the snippet used to point at
    // `{site}/public/signup-form.min.js`, which this server does not
    // serve, so every embed handed out was a 404.
    let html = embed("all-in-one", &[]);
    assert!(html.contains(&ghost_render::apps::signup_form_script_url()));
    assert!(!html.contains("/public/signup-form"));
    // The script has to be in a sized box or the form renders into
    // nothing, and `async` is how Ghost loads it.
    assert!(html.starts_with(r#"<div style="height: 40vmin;min-height: 360px">"#));
    assert!(html.contains(" async></script></div>"));
}

#[test]
fn the_embed_sends_both_computed_contrast_colours() {
    // Without these the copy is drawn dark on a dark background.
    let html = embed("all-in-one", &[]);
    assert!(html.contains(r##"data-background-color="#ffffff""##));
    assert!(html.contains(r##"data-text-color="#000000""##), "white needs dark text");
    assert!(html.contains(r##"data-button-text-color="#FFFFFF""##), "a dark button needs light text");
}

#[test]
fn the_minimal_layout_drops_the_copy_and_the_background() {
    // Minimal is meant to sit on whatever is behind it, so Ghost sends
    // no background at all — and no title, description or icon.
    let html = embed("minimal", &["a", "b"]);
    for absent in ["data-background-color", "data-text-color", "data-title", "data-description", "data-icon"] {
        assert!(!html.contains(absent), "{absent} does not belong on the minimal layout");
    }
    assert!(html.contains(r##"data-button-color="#15171A""##));
    assert!(html.contains(r#"data-label-1="a""#) && html.contains(r#"data-label-2="b""#));
    assert!(html.contains(r#"data-site="https://site.test/""#), "the site URL carries its trailing slash");
}

#[test]
fn embed_attributes_follow_ghosts_order() {
    // Ghost sorts by a fixed preferred order, so a snippet from here and
    // one from Ghost diff cleanly rather than line-by-line.
    let html = embed("all-in-one", &["x"]);
    let order = [
        "data-background-color", "data-text-color", "data-button-color",
        "data-button-text-color", "data-title", "data-description", "data-icon",
        "data-site", "data-locale", "data-label-1",
    ];
    let mut last = 0;
    for key in order {
        let at = html.find(key).unwrap_or_else(|| panic!("{key} missing"));
        assert!(at > last, "{key} is out of order");
        last = at;
    }
}

#[test]
fn the_embed_icon_is_asked_for_at_192_square() {
    assert_eq!(
        icon_at_192("https://s.test/content/images/2026/01/icon.png"),
        "https://s.test/content/images/size/w192h192/2026/01/icon.png"
    );
    // Any size already on the URL is replaced, not stacked.
    assert_eq!(
        icon_at_192("https://s.test/content/images/size/w256h256/2026/01/icon.png"),
        "https://s.test/content/images/size/w192h192/2026/01/icon.png"
    );
    // A URL that is not a content image is left exactly as it is.
    assert_eq!(icon_at_192("https://cdn.test/logo.svg"), "https://cdn.test/logo.svg");
}

#[test]
fn the_preview_embed_fills_its_frame() {
    // Same attributes, different box: the preview is the real snippet
    // sized to the frame rather than to a page.
    let page = embed("all-in-one", &[]);
    let preview = signup_form_snippet(SignupFormEmbed {
        site_url: "https://site.test", layout: "all-in-one", background: "#ffffff",
        button: "#15171A", title: "My Site", description: "A description",
        icon: "/content/images/2026/01/icon.png", locale: "en", labels: &[], preview: true,
    });
    assert!(preview.contains(r#"style="height: 100vh""#));
    assert_ne!(page, preview);
    // The data attributes are identical — only the wrapper differs.
    let strip = |s: &str| s[s.find("<script").unwrap()..].to_string();
    assert_eq!(strip(&page), strip(&preview));
}
