//! The internet-facing half of Advertising & Ad Management.
//!
//! In the order a page view meets it:
//!
//! 1. **[`plan`]** chooses, once per request, which ad fills each placement
//!    for this reader on this page: live ads, filtered by
//!    `ghost_core::ads::eligible` (schedule, caps, audience, targeting,
//!    paying members), then one per placement by weighted rotation.
//! 2. **[`Plan::apply_to_post`]** puts the in-content ads into the post's
//!    body *before* the theme renders it — top, after paragraph N, end — so
//!    it works with every theme without knowing its markup.
//! 3. **[`Plan::finish`]** adds what belongs to the whole page after
//!    rendering: the sticky bar, any `<div data-ad-slot="name"></div>` an
//!    author or theme placed, and the stylesheet and script — only when an
//!    ad was actually placed.
//! 4. **`POST /ads/api/events/`** counts what the script saw: an impression
//!    once half the ad has been on screen for a second, a click when one of
//!    its links is followed. Same-origin, a 1 KB body, at most twelve ids,
//!    and only ids of live ads move a counter.
//! 5. **`GET /ads.txt`** serves the seller list the tool keeps, while ads
//!    are on and the list is not empty; otherwise a 404 as before.
//!
//! An ad's link carries `rel="sponsored noopener"`: search engines are told
//! it is paid, and the advertiser's page cannot reach back into this one.
//! Code ads are staff-pasted HTML, trusted like Code injection; the consent
//! module, which runs after this on the same page, still holds back any
//! tracker inside them until the visitor agrees.

use crate::AppState;
use axum::{
    extract::{DefaultBodyLimit, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use ghost_core::ads::{self as rules, Device, Kind, Placement, SiteSettings, Viewer};
use ghost_db::repos::ads::{self as repo, Ad};
use serde_json::Value;
use std::collections::HashMap;

const MAX_BODY_BYTES: usize = 1024;

pub static JS: &str = include_str!("../assets/ads.js");
pub static CSS: &str = include_str!("../assets/ads.css");

fn asset_version() -> &'static str {
    static V: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        let mut hash: u64 = 0xcbf29ce484222325;
        for byte in JS.bytes().chain(CSS.bytes()) {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        format!("{hash:x}")
    });
    &V
}

pub fn public_router() -> Router<AppState> {
    Router::new()
        .route("/ads/api/events/", post(events).layer(DefaultBodyLimit::max(MAX_BODY_BYTES)))
        .route("/ads.txt", get(ads_txt))
        .route("/ads/api/assets/{version}/ads.js", get(|| async { asset(JS, "text/javascript; charset=utf-8") }))
        .route("/ads/api/assets/{version}/ads.css", get(|| async { asset(CSS, "text/css; charset=utf-8") }))
}

fn asset(body: &'static str, mime: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        body,
    )
        .into_response()
}

pub fn settings(state: &AppState) -> SiteSettings {
    SiteSettings::read(|key| state.settings.get_str(key))
}

// ---------------------------------------------------------------------------
// choosing
// ---------------------------------------------------------------------------

/// The ads chosen for one page view.
#[derive(Debug, Default)]
pub struct Plan {
    pub top: Option<Ad>,
    pub after: Vec<(i64, Ad)>,
    pub bottom: Option<Ad>,
    pub sticky: Option<Ad>,
    pub slots: HashMap<String, Ad>,
    label: String,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.top.is_none() && self.after.is_empty() && self.bottom.is_none() && self.sticky.is_none() && self.slots.is_empty()
    }
}

/// Which ad fills each placement, for this reader on this page. `None` when
/// ads are off or nothing is eligible — the common case, and then the page
/// is not touched at all.
pub async fn plan(state: &AppState, path: &str, member: &Value, kind: &str, tags: &[String]) -> Option<Plan> {
    let site = settings(state);
    if !site.enabled {
        return None;
    }
    let ads = repo::live(&state.db).await.ok()?;
    if ads.is_empty() {
        return None;
    }
    let now = ghost_core::dates::now_db();
    let status = member.get("status").and_then(Value::as_str);
    let viewer = Viewer { member: status, path, tags, kind, now: &now };
    let rule_lists: Vec<Vec<String>> = ads.iter().map(Ad::rule_list).collect();
    let eligible: Vec<&Ad> = ads
        .iter()
        .zip(rule_lists.iter())
        .filter(|(ad, rl)| rules::eligible(&rule_of(ad, rl), &viewer, &site))
        .map(|(ad, _)| ad)
        .collect();
    if eligible.is_empty() {
        return None;
    }
    let choose = |candidates: Vec<&Ad>| -> Option<Ad> {
        let weighted: Vec<(&Ad, i64)> = candidates.into_iter().map(|a| (a, a.weight)).collect();
        rules::pick(&weighted, rand::random::<u64>()).map(|a| (*a).clone())
    };
    let of = |p: Placement| eligible.iter().copied().filter(move |a| Placement::from_key(&a.placement) == p);
    let mut plan = Plan { label: site.label.clone(), ..Plan::default() };
    plan.top = choose(of(Placement::PostTop).collect());
    plan.bottom = choose(of(Placement::PostBottom).collect());
    plan.sticky = choose(of(Placement::Sticky).collect());
    // One ad per paragraph position, as many positions as the site allows
    // in one body once top and bottom are counted.
    let mut positions: Vec<i64> = of(Placement::PostParagraph).map(|a| a.paragraph).collect();
    positions.sort_unstable();
    positions.dedup();
    let used = plan.top.is_some() as i64 + plan.bottom.is_some() as i64;
    for n in positions.into_iter().take((site.max_in_content - used).max(0) as usize) {
        if let Some(ad) = choose(of(Placement::PostParagraph).filter(|a| a.paragraph == n).collect()) {
            plan.after.push((n, ad));
        }
    }
    let mut slot_names: Vec<String> = of(Placement::Slot).map(|a| a.slot.clone()).collect();
    slot_names.sort();
    slot_names.dedup();
    for name in slot_names {
        if let Some(ad) = choose(of(Placement::Slot).filter(|a| a.slot == name).collect()) {
            plan.slots.insert(name, ad);
        }
    }
    (!plan.is_empty()).then_some(plan)
}

pub fn rule_of<'a>(ad: &'a Ad, rule_list: &'a [String]) -> rules::Rule<'a> {
    rules::Rule {
        status: rules::Status::from_key(&ad.status),
        placement: Placement::from_key(&ad.placement),
        audience: rules::Audience::from_key(&ad.audience),
        targeting: rules::Targeting::from_key(&ad.targeting),
        rules: rule_list,
        starts_at: ad.starts_at.as_deref(),
        ends_at: ad.ends_at.as_deref(),
        max_impressions: ad.max_impressions,
        max_clicks: ad.max_clicks,
        impressions: ad.impressions,
        clicks: ad.clicks,
    }
}

impl Plan {
    /// Put the in-content ads into the post body the theme is about to
    /// render. A page without a `post.html` is left alone.
    pub fn apply_to_post(&self, data: &mut Value) {
        let Some(html) = data.pointer("/post/html").and_then(Value::as_str) else { return };
        let top = self.top.as_ref().map(|a| markup(a, &self.label));
        let bottom = self.bottom.as_ref().map(|a| markup(a, &self.label));
        let after: Vec<(i64, String)> = self.after.iter().map(|(n, a)| (*n, markup(a, &self.label))).collect();
        if top.is_none() && bottom.is_none() && after.is_empty() {
            return;
        }
        let placed = rules::place_in_content(html, top.as_deref(), &after, bottom.as_deref());
        data["post"]["html"] = Value::String(placed);
    }

    /// The whole-page half: sticky bar, slots, stylesheet, script.
    pub fn finish(&self, html: String) -> String {
        let mut html = html;
        for (name, ad) in &self.slots {
            html = fill_slot(&html, name, &markup(ad, &self.label));
        }
        let mut tail = String::new();
        if let Some(ad) = &self.sticky {
            tail.push_str(&format!(
                r#"<div class="vad-sticky" data-ad-sticky>{inner}<button type="button" class="vad-close" aria-label="Close advertisement">×</button></div>"#,
                inner = markup(ad, &self.label),
            ));
        }
        let base = ghost_core::url::base("");
        let v = asset_version();
        tail.push_str(&format!(
            r#"<script src="{base}/ads/api/assets/{v}/ads.js" defer data-cookie-ignore data-ads="{base}/ads/api/events/"></script>"#
        ));
        let css = format!(r#"<link rel="stylesheet" href="{base}/ads/api/assets/{v}/ads.css">"#);
        if let Some(at) = html.find("</head>") {
            html.insert_str(at, &css);
        } else {
            tail.insert_str(0, &css);
        }
        match html.rfind("</body>") {
            Some(at) => {
                html.insert_str(at, &tail);
                html
            }
            None => html,
        }
    }
}

/// Put an ad inside every `<… data-ad-slot="name">` on the page, right after
/// its opening tag. The slot's own markup (a theme's sidebar wrapper, say)
/// is kept around it.
fn fill_slot(html: &str, name: &str, ad: &str) -> String {
    let needle = format!(r#"data-ad-slot="{name}""#);
    let mut out = String::with_capacity(html.len() + ad.len());
    let mut rest = html;
    while let Some(at) = rest.find(&needle) {
        let Some(close) = rest[at..].find('>') else { break };
        let end = at + close + 1;
        out.push_str(&rest[..end]);
        out.push_str(ad);
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------------------
// markup
// ---------------------------------------------------------------------------

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// One ad, labelled. Every value is escaped except a code ad's own HTML.
pub fn markup(ad: &Ad, label: &str) -> String {
    let device = match Device::from_key(&ad.device) {
        Device::All => "",
        Device::Desktop => " vad-desktop",
        Device::Mobile => " vad-mobile",
    };
    let placement = Placement::from_key(&ad.placement).key();
    let inner = match Kind::from_key(&ad.kind) {
        Kind::Image => format!(
            r#"<a class="vad-link" href="{link}" rel="sponsored noopener" target="_blank"><img class="vad-img" src="{img}" alt="{alt}" loading="lazy" decoding="async"></a>"#,
            link = esc(&ad.link),
            img = esc(&ad.image),
            alt = esc(&ad.alt),
        ),
        Kind::Text => format!(
            r#"<a class="vad-link vad-text" href="{link}" rel="sponsored noopener" target="_blank">{img}<span class="vad-words"><b class="vad-headline">{h}</b>{b}</span>{cta}</a>"#,
            link = esc(&ad.link),
            img = if ad.image.is_empty() { String::new() } else { format!(r#"<img class="vad-thumb" src="{}" alt="" loading="lazy">"#, esc(&ad.image)) },
            h = esc(&ad.headline),
            b = if ad.body.is_empty() { String::new() } else { format!(r#"<span class="vad-body">{}</span>"#, esc(&ad.body)) },
            cta = if ad.cta.is_empty() { String::new() } else { format!(r#"<span class="vad-cta">{}</span>"#, esc(&ad.cta)) },
        ),
        Kind::Code => format!(r#"<div class="vad-code">{}</div>"#, ad.code),
    };
    format!(
        r#"<aside class="vad vad-{placement}{device}" data-ad="{id}" aria-label="{label}"><span class="vad-label">{label}</span>{inner}</aside>"#,
        id = esc(&ad.id),
        label = esc(label),
    )
}

// ---------------------------------------------------------------------------
// counting and ads.txt
// ---------------------------------------------------------------------------

/// `i=id,id&c=id`. Ids that are not the shape the site issues are dropped
/// before the database sees them.
async fn events(State(state): State<AppState>, headers: HeaderMap, body: String) -> Response {
    if !crate::forms::same_origin(&headers, &state.site_url()) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !settings(&state).enabled {
        return StatusCode::NO_CONTENT.into_response();
    }
    let mut seen: Vec<String> = Vec::new();
    let mut clicked: Vec<String> = Vec::new();
    for (k, v) in form_urlencoded::parse(body.as_bytes()) {
        let ids = v
            .split(',')
            .map(str::trim)
            .filter(|id| ghost_core::ids::is_objectid(id))
            .take(rules::MAX_EVENTS)
            .map(str::to_string);
        match k.as_ref() {
            "i" => seen.extend(ids),
            "c" => clicked.extend(ids),
            _ => {}
        }
    }
    seen.sort();
    seen.dedup();
    clicked.sort();
    clicked.dedup();
    repo::count(&state.db, &seen, false).await;
    repo::count(&state.db, &clicked, true).await;
    StatusCode::NO_CONTENT.into_response()
}

async fn ads_txt(State(state): State<AppState>) -> Response {
    let site = settings(&state);
    if !site.enabled || site.ads_txt.is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }
    (
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        format!("{}\n", site.ads_txt),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ad(kind: &str) -> Ad {
        Ad {
            id: "aaaaaaaaaaaaaaaaaaaaaaaa".into(), advertiser_id: None, name: "x".into(), status: "live".into(),
            kind: kind.into(), placement: "post_bottom".into(), paragraph: 2, slot: "".into(),
            image: "https://cdn.example/a.png".into(), link: "https://shop.example/?a=1&b=2".into(),
            alt: "\"><script>".into(), headline: "<b>Buy</b>".into(), body: "".into(), cta: "Go".into(),
            code: "<ins class=\"adsbygoogle\"></ins>".into(), audience: "everyone".into(), device: "mobile".into(),
            targeting: "everywhere".into(), rules: "".into(), weight: 1, starts_at: None, ends_at: None,
            max_impressions: 0, max_clicks: 0, cpm_cents: 0, cpc_cents: 0, impressions: 0, clicks: 0,
            created_at: "".into(), updated_at: "".into(),
        }
    }

    #[test]
    fn an_ad_is_labelled_escaped_and_marked_sponsored() {
        let html = markup(&ad("image"), "Sponsored <x>");
        assert!(html.contains(r#"<span class="vad-label">Sponsored &lt;x&gt;</span>"#), "{html}");
        assert!(html.contains(r#"rel="sponsored noopener""#));
        assert!(html.contains("a=1&amp;b=2"));
        assert!(!html.contains("<script>"));
        assert!(html.contains("vad-mobile"));
        let text = markup(&ad("text"), "Ad");
        assert!(text.contains("&lt;b&gt;Buy&lt;/b&gt;"));
        // A code ad is the network's own HTML, placed as pasted.
        assert!(markup(&ad("code"), "Ad").contains(r#"<ins class="adsbygoogle"></ins>"#));
    }

    #[test]
    fn a_slot_keeps_its_wrapper_and_gets_the_ad_inside() {
        let out = fill_slot(r#"<div class="side" data-ad-slot="sidebar"></div><p data-ad-slot="other"></p>"#, "sidebar", "[AD]");
        assert_eq!(out, r#"<div class="side" data-ad-slot="sidebar">[AD]</div><p data-ad-slot="other"></p>"#);
    }
}
