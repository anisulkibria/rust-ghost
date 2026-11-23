//! The ads repository, on SQLite by default and on MySQL when
//! GHOST_TEST_MYSQL_URL is set.
//!
//! The save-then-read below changes every field, because a write whose
//! column list and bind list disagree reports success and stores nothing —
//! the bug that once made every Popups save a no-op.

use vuuut_core::ads::AdInput;
use vuuut_db::pool::connect_test;
use vuuut_db::repos::ads as repo;
use std::collections::HashMap;

fn input(pairs: &[(&str, &str)]) -> AdInput {
    let form: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    AdInput::from_form(&form, |_| true).unwrap()
}

#[tokio::test]
async fn an_ad_survives_a_save_with_every_field_changed() {
    let db = connect_test().await.unwrap();
    let pool = db.pool();
    let adv = repo::save_advertiser(&pool, None, "Acme", "ads@acme.test", "https://acme.test", "net 30").await.unwrap();
    let id = repo::insert(&pool, &input(&[("name", "First"), ("kind", "code"), ("code", "<b>x</b>")])).await.unwrap();
    let changed = input(&[
        ("name", "Second"), ("advertiser_id", &adv), ("status", "live"), ("kind", "text"),
        ("placement", "post_paragraph"), ("paragraph", "4"), ("slot", "side"), ("image", "/i.png"),
        ("link", "https://acme.test/x"), ("alt", "Alt"), ("headline", "Head"), ("body", "Body"), ("cta", "Go"),
        ("code", "<i>y</i>"), ("audience", "not_paid"), ("device", "mobile"), ("targeting", "only"),
        ("rules", "/guides/\ntag:news"), ("weight", "7"), ("starts_at", "2026-11-01"), ("ends_at", "2026-12-31"),
        ("max_impressions", "1000"), ("max_clicks", "50"), ("cpm", "3.25"), ("cpc", "0.40"),
    ]);
    repo::update(&pool, &id, &changed).await.unwrap();
    let a = repo::find(&pool, &id).await.unwrap().unwrap();
    assert_eq!(
        (a.name.as_str(), a.advertiser_id.as_deref(), a.status.as_str(), a.kind.as_str(), a.placement.as_str(), a.paragraph),
        ("Second", Some(adv.as_str()), "live", "text", "post_paragraph", 4)
    );
    assert_eq!((a.slot.as_str(), a.image.as_str(), a.link.as_str(), a.alt.as_str()), ("side", "/i.png", "https://acme.test/x", "Alt"));
    assert_eq!((a.headline.as_str(), a.body.as_str(), a.cta.as_str(), a.code.as_str()), ("Head", "Body", "Go", "<i>y</i>"));
    assert_eq!((a.audience.as_str(), a.device.as_str(), a.targeting.as_str()), ("not_paid", "mobile", "only"));
    assert_eq!(a.rule_list(), ["/guides/", "tag:news"]);
    assert_eq!((a.weight, a.max_impressions, a.max_clicks, a.cpm_cents, a.cpc_cents), (7, 1000, 50, 325, 40));
    assert_eq!(a.starts_at.as_deref(), Some("2026-11-01 00:00:00"));
    assert_eq!(a.ends_at.as_deref(), Some("2026-12-31 23:59:59"));
    assert_eq!(repo::of_advertiser(&pool, &adv).await.unwrap().len(), 1);
}

#[tokio::test]
async fn only_live_ads_are_counted_and_the_days_add_up() {
    let db = connect_test().await.unwrap();
    let pool = db.pool();
    let live = repo::insert(&pool, &input(&[("name", "Live"), ("status", "live"), ("kind", "code"), ("code", "x")])).await.unwrap();
    let draft = repo::insert(&pool, &input(&[("name", "Draft"), ("kind", "code"), ("code", "x")])).await.unwrap();
    repo::count(&pool, &[live.clone(), draft.clone(), "ffffffffffffffffffffffff".into()], false).await;
    repo::count(&pool, std::slice::from_ref(&live), false).await;
    repo::count(&pool, std::slice::from_ref(&live), true).await;
    let a = repo::find(&pool, &live).await.unwrap().unwrap();
    assert_eq!((a.impressions, a.clicks), (2, 1));
    assert_eq!(repo::find(&pool, &draft).await.unwrap().unwrap().impressions, 0);
    let days = repo::daily(&pool, "2000-01-01", None).await.unwrap();
    assert_eq!(days.len(), 1);
    assert_eq!((days[0].1, days[0].2), (2, 1));
    assert_eq!(repo::totals_since(&pool, "2000-01-01").await.unwrap(), vec![(live.clone(), 2, 1)]);
    repo::destroy(&pool, &live).await.unwrap();
    assert!(repo::daily(&pool, "2000-01-01", None).await.unwrap().is_empty());
}

#[tokio::test]
async fn deleting_an_advertiser_keeps_their_ads_as_house_ads() {
    let db = connect_test().await.unwrap();
    let pool = db.pool();
    let adv = repo::save_advertiser(&pool, None, "Acme", "", "", "").await.unwrap();
    let id = repo::insert(&pool, &input(&[("name", "A"), ("advertiser_id", &adv), ("kind", "code"), ("code", "x")])).await.unwrap();
    repo::delete_advertiser(&pool, &adv).await.unwrap();
    assert!(repo::advertiser(&pool, &adv).await.unwrap().is_none());
    assert_eq!(repo::find(&pool, &id).await.unwrap().unwrap().advertiser_id, None);
}
