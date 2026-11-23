//! Tools → Page Builder on the public site.
//!
//! * [`expand`] — the request-time pass over stored layout HTML: display
//!   conditions for this visitor, dynamic tags, and dynamic widgets (posts,
//!   menus, breadcrumbs, author box, post navigation, sitemap, login, logo).
//!   The admin's canvas calls the same function, so a preview is exact.
//! * [`site_parts`] — middleware putting the Theme Builder's header, footer
//!   and popups into every HTML page they apply to.
//! * [`maintenance`] — middleware answering every visitor (staff excepted)
//!   with the chosen page while coming-soon or maintenance mode is on.
//! * [`entry_html`] — what a post's body becomes: its own layout, else a
//!   matching single-post template wrapped around its content.

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, Response, StatusCode};
use axum::response::IntoResponse;
use vuuut_api::AppState;
use vuuut_core::builder::{self as b, dynamic, esc, parts::PartOptions};
use vuuut_db::repos;
use serde_json::{json, Value};

pub fn settings(state: &AppState) -> b::Settings {
    b::Settings::read(|k| state.settings.get_str(k))
}

fn hash_of(s: &str) -> String {
    use sha2::Digest;
    hex::encode(&sha2::Sha256::digest(s.as_bytes())[..6])
}

/// The stylesheet and runtime every builder page carries, versioned by
/// content so they can be cached for good.
pub fn asset_tags() -> String {
    static TAGS: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        format!(
            r#"<link rel="stylesheet" href="{}?v={}"><script src="{}?v={}" defer></script>"#,
            vuuut_core::url::base("/public/vuuut-builder.css"),
            hash_of(b::CSS),
            vuuut_core::url::base("/public/vuuut-builder.js"),
            hash_of(b::RUNTIME)
        )
    });
    TAGS.clone()
}

/// What a post or page's body is drawn from, if the builder has a say:
/// `(html, from_template)`. A template carries a Post content widget that
/// [`expand`] fills with the post's own body.
pub(crate) async fn entry_html(state: &AppState, post: &vuuut_core::models::Post) -> Option<(String, bool)> {
    let s = settings(state);
    if !s.allows(&post.post_type) || post.id.starts_with("virtual") {
        return None;
    }
    if let Some(html) = repos::builder::active_html(&state.db, &post.id).await {
        return Some((html, false));
    }
    let singles = repos::builder::active_parts(&state.db, "single").await;
    if singles.is_empty() {
        return None;
    }
    let rel = repos::posts::load_relations(&state.db, std::slice::from_ref(post)).await.unwrap_or_default();
    let tags: Vec<String> = rel.tags_for(&post.id).iter().filter_map(|t| t.slug.clone()).collect();
    let path = format!("/{}/", post.slug.as_deref().unwrap_or(""));
    singles
        .into_iter()
        .map(|(id, html, opts)| (id, html, PartOptions::from_value(&serde_json::from_str(&opts).unwrap_or(json!({})))))
        .filter(|(_, _, o)| o.applies_to_post(&post.post_type, &tags, &path))
        .max_by_key(|(_, _, o)| o.priority)
        .map(|(_, html, _)| (html, true))
}

fn fmt_date(s: &str) -> String {
    let s = s.trim();
    let parsed = chrono::NaiveDateTime::parse_from_str(&s[..19.min(s.len())].replace('T', " "), "%Y-%m-%d %H:%M:%S")
        .ok()
        .or_else(|| chrono::NaiveDate::parse_from_str(&s[..10.min(s.len())], "%Y-%m-%d").ok().and_then(|d| d.and_hms_opt(0, 0, 0)));
    parsed.map(|d| d.format("%-d %b %Y").to_string()).unwrap_or_default()
}

fn member_state(member: &Value) -> &str {
    if member.is_null() {
        ""
    } else {
        member["status"].as_str().unwrap_or("free")
    }
}

/// One dynamic widget, drawn.
async fn draw(state: &AppState, kind: &str, id: &str, q: &Value, post: Option<&Value>, original: Option<&str>, member: &Value, path: &str) -> String {
    let st = |k: &str| q[k].as_str().unwrap_or("");
    let flag = |k: &str| q[k].as_bool().unwrap_or(false);
    let al = match st("align") { "" => "left", a => a };
    match kind {
        "post-content" => match (original, post) {
            (Some(h), _) => format!(r#"<div class="vb-post-content gh-content">{h}</div>"#),
            (None, Some(p)) if p["html"].is_string() => String::new(),
            _ => r#"<div class="vb-post-content"><p style="opacity:.6">The post's own content appears here.</p></div>"#.into(),
        },
        "nav-menu" => {
            let key = if st("menu") == "secondary" { "secondary_navigation" } else { "navigation" };
            let nav = state.settings.get(key).unwrap_or(Value::Null);
            let nav = match nav {
                Value::String(s) => serde_json::from_str(&s).unwrap_or(Value::Null),
                v => v,
            };
            let items: String = nav
                .as_array()
                .into_iter()
                .flatten()
                .map(|i| {
                    let url = i["url"].as_str().unwrap_or("/");
                    let href = if url.starts_with("http") || url.starts_with('#') || url.starts_with("mailto:") { url.to_string() } else { vuuut_core::url::base(url) };
                    let here = url.trim_end_matches('/') == path.trim_end_matches('/') || (url == "/" && path == "/");
                    format!(r#"<li><a href="{}"{}>{}</a></li>"#, esc(&href), if here { r#" aria-current="page""# } else { "" }, esc(i["label"].as_str().unwrap_or("")))
                })
                .collect();
            format!(
                r#"<nav class="vb-menu vb-menu-{lay} vb-ptr-{ptr} vb-mt-{mt}" aria-label="Menu"><button type="button" class="vb-menu-toggle" aria-expanded="false" aria-label="Menu">{ic}</button><ul>{items}</ul></nav>"#,
                lay = if st("layout") == "vertical" { "v" } else { "h" },
                ptr = match st("pointer") { "" => "underline", p => p },
                mt = match st("mobile") { "" => "tablet", m => m },
                ic = b::icon_svg("menu").unwrap_or_default(),
            )
        }
        "site-logo" => {
            let title = state.settings.get_str("title").unwrap_or_default();
            let logo = state.settings.get_str("logo").filter(|l| !l.is_empty());
            let icon = state.settings.get_str("icon").filter(|l| !l.is_empty());
            let show = match st("show") { "" => "auto", s => s };
            let inner = match (show, &logo, &icon) {
                ("icon", _, Some(i)) => format!(r#"<img src="{}" alt="{}">"#, esc(&vuuut_core::url::base_asset(i)), esc(&title)),
                ("title", _, _) | (_, None, _) | ("icon", _, None) => format!("<b>{}</b>", esc(&title)),
                (_, Some(l), _) => format!(r#"<img src="{}" alt="{}">"#, esc(&vuuut_core::url::base_asset(l)), esc(&title)),
            };
            let inner = if q["link"].as_bool().unwrap_or(true) { format!(r#"<a href="{}">{inner}</a>"#, esc(&vuuut_core::url::base("/"))) } else { inner };
            format!(r#"<div class="vb-logo">{inner}</div>"#)
        }
        "login" => {
            let btn = |label: &str, href: &str, style: &str| format!(r#"<a class="vb-btn vb-btn-{style} vb-btn-s" href="{href}">{}</a>"#, esc(label));
            let inner = if member.is_null() {
                format!("{}{}", btn(match st("signin") { "" => "Sign in", s => s }, "#/portal/signin", "link"), btn(match st("signup") { "" => "Subscribe", s => s }, "#/portal/signup", "filled"))
            } else {
                btn(match st("account") { "" => "Account", s => s }, "#/portal/account", "outline")
            };
            let style = q["colour"].as_str().map(|c| format!(r#" style="--vb-bc:{}""#, esc(&vuuut_core::builder::render::css_colour(c)))).unwrap_or_default();
            format!(r#"<div class="vb-login vb-al-{al}"{style}>{inner}</div>"#)
        }
        "breadcrumbs" => {
            let sep = esc(match st("separator") { "" => "›", s => s });
            let home = match st("home") { "" => "Home", h => h };
            let mut crumbs = vec![(home.to_string(), vuuut_core::url::base("/"))];
            if let Some(p) = post {
                if let Some(t) = p["primary_tag"].as_object() {
                    crumbs.push((t.get("name").and_then(Value::as_str).unwrap_or("").to_string(), t.get("url").and_then(Value::as_str).unwrap_or("").to_string()));
                }
                crumbs.push((p["title"].as_str().unwrap_or("").to_string(), String::new()));
            } else {
                let mut acc = String::new();
                let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
                for (i, seg) in segs.iter().enumerate() {
                    acc.push('/');
                    acc.push_str(seg);
                    let label = seg.replace('-', " ");
                    crumbs.push((label, if i + 1 == segs.len() { String::new() } else { format!("{acc}/") }));
                }
            }
            let items: String = crumbs
                .iter()
                .enumerate()
                .map(|(i, (l, u))| {
                    let inner = if u.is_empty() { format!(r#"<span aria-current="page">{}</span>"#, esc(l)) } else { format!(r#"<a href="{}">{}</a>"#, esc(u), esc(l)) };
                    format!(r#"<li data-sep="{}">{inner}</li>"#, if i == 0 { String::new() } else { sep.clone() })
                })
                .collect();
            let schema = if flag("schema") {
                let list: Vec<Value> = crumbs.iter().enumerate().map(|(i, (l, u))| json!({"@type": "ListItem", "position": i + 1, "name": l, "item": if u.is_empty() { Value::Null } else { json!(u) }})).collect();
                format!(r#"<script type="application/ld+json">{}</script>"#, json!({"@context": "https://schema.org", "@type": "BreadcrumbList", "itemListElement": list}).to_string().replace("</", "<\\/"))
            } else {
                String::new()
            };
            format!(r#"<nav class="vb-crumbs" aria-label="Breadcrumb" style="text-align:{al}"><ol>{items}</ol></nav>{schema}"#)
        }
        "author-box" => {
            let Some(a) = post.and_then(|p| p["primary_author"].as_object()) else {
                return r#"<div class="vb-author"><p style="opacity:.6">The post's author appears here.</p></div>"#.into();
            };
            let name = a.get("name").and_then(Value::as_str).unwrap_or("");
            let url = a.get("url").and_then(Value::as_str).unwrap_or("");
            let img = if q["avatar"].as_bool().unwrap_or(true) { a.get("profile_image").and_then(Value::as_str).filter(|s| !s.is_empty()).map(|i| format!(r#"<img src="{}" alt="">"#, esc(i))).unwrap_or_default() } else { String::new() };
            let bio = if q["bio"].as_bool().unwrap_or(true) { a.get("bio").and_then(Value::as_str).map(|t| format!("<p>{}</p>", esc(t))).unwrap_or_default() } else { String::new() };
            let name_html = if q["link"].as_bool().unwrap_or(true) && !url.is_empty() { format!(r#"<a href="{}">{}</a>"#, esc(url), esc(name)) } else { esc(name) };
            format!(
                r#"<div class="vb-author vb-author-{}{}">{img}<div><b>{name_html}</b>{bio}</div></div>"#,
                if st("layout") == "top" { "top" } else { "left" },
                if flag("boxed") { " vb-boxed-a" } else { "" }
            )
        }
        "post-info" => {
            let Some(p) = post else { return r#"<div class="vb-postinfo"><span>Author · Date · 4 min read</span></div>"#.into() };
            let sep = esc(match st("separator") { "" => "·", s => s });
            let mut parts = Vec::new();
            if q["author"].as_bool().unwrap_or(true) {
                if let Some(a) = p["primary_author"].as_object() {
                    parts.push(format!(r#"<a href="{}">{}</a>"#, esc(a.get("url").and_then(Value::as_str).unwrap_or("")), esc(a.get("name").and_then(Value::as_str).unwrap_or(""))));
                }
            }
            if q["date"].as_bool().unwrap_or(true) {
                parts.push(esc(&fmt_date(p["published_at"].as_str().unwrap_or(""))));
            }
            if flag("updated") {
                parts.push(format!("Updated {}", esc(&fmt_date(p["updated_at"].as_str().unwrap_or("")))));
            }
            if q["reading"].as_bool().unwrap_or(true) {
                parts.push(format!("{} min read", p["reading_time"].as_i64().unwrap_or(1).max(1)));
            }
            if flag("tags") {
                let tags: Vec<String> = p["tags"].as_array().into_iter().flatten().map(|t| format!(r#"<a href="{}">{}</a>"#, esc(t["url"].as_str().unwrap_or("")), esc(t["name"].as_str().unwrap_or("")))).collect();
                if !tags.is_empty() {
                    parts.push(tags.join(", "));
                }
            }
            let items: String = parts.iter().enumerate().map(|(i, x)| format!(r#"<span data-sep="{}">{x}</span>"#, if i == 0 { String::new() } else { sep.clone() })).collect();
            format!(r#"<div class="vb-postinfo vb-al-{al}">{items}</div>"#)
        }
        "post-nav" => {
            let Some(p) = post else { return r##"<nav class="vb-postnav"><a href="#"><span><small>Previous</small>An earlier post</span></a><a class="vb-pn-next" href="#"><span><small>Next</small>A later post</span></a></nav>"##.into() };
            let at = p["published_at"].as_str().unwrap_or("").replace('T', " ");
            let at = at.trim_end_matches('Z').to_string();
            let kind = p["type"].as_str().unwrap_or("post");
            let near = |op: &'static str, ord: &'static str| {
                let at = at.clone();
                async move {
                    sqlx::query_scalar::<_, String>(&format!("SELECT slug FROM posts WHERE type = ? AND status = 'published' AND published_at {op} ? ORDER BY published_at {ord} LIMIT 1"))
                        .bind(kind)
                        .bind(at)
                        .fetch_optional(&state.db)
                        .await
                        .ok()
                        .flatten()
                }
            };
            let resolver = crate::resolver_now(state).await;
            let mut out = String::new();
            for (slug, cls, label) in [(near("<", "DESC").await, "vb-pn-prev", match st("prev") { "" => "Previous", s => s }), (near(">", "ASC").await, "vb-pn-next", match st("next") { "" => "Next", s => s })] {
                let Some(slug) = slug else { continue };
                let Ok(Some(found)) = repos::posts::find_by_slug(&state.db, &slug, kind, true).await else { continue };
                let j = vuuut_core::models::post_to_json(&found, &[], &[], "");
                let url = resolver(&j);
                let img = if flag("images") { found.feature_image.as_deref().map(|i| format!(r#"<img src="{}" alt="">"#, esc(&vuuut_core::url::base_asset(i)))).unwrap_or_default() } else { String::new() };
                let title = if q["titles"].as_bool().unwrap_or(true) { esc(found.title.as_deref().unwrap_or("")) } else { String::new() };
                out.push_str(&format!(r#"<a class="{cls}" href="{}">{img}<span><small>{}</small>{title}</span></a>"#, esc(&url), esc(label)));
            }
            format!(r#"<nav class="vb-postnav" aria-label="More posts">{out}</nav>"#)
        }
        "sitemap" => {
            let limit = q["limit"].as_i64().unwrap_or(100).clamp(1, 500);
            let source = match st("source") { "" => "pages", s => s };
            let rows: Vec<(String, String)> = match source {
                "tags" => sqlx::query_as::<_, (Option<String>, String)>("SELECT name, slug FROM tags WHERE visibility = 'public' ORDER BY name LIMIT ?")
                    .bind(limit)
                    .fetch_all(&state.db)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(n, s)| (n.unwrap_or_default(), vuuut_core::url::base(&format!("/tag/{s}/"))))
                    .collect(),
                "authors" => sqlx::query_as::<_, (String, String)>("SELECT name, slug FROM users WHERE status != 'inactive' ORDER BY name LIMIT ?")
                    .bind(limit)
                    .fetch_all(&state.db)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(n, s)| (n, vuuut_core::url::base(&format!("/author/{s}/"))))
                    .collect(),
                _ => {
                    let kind = if source == "posts" { "post" } else { "page" };
                    let slugs: Vec<String> = sqlx::query_scalar("SELECT slug FROM posts WHERE type = ? AND status = 'published' AND visibility = 'public' ORDER BY title LIMIT ?")
                        .bind(kind)
                        .bind(limit)
                        .fetch_all(&state.db)
                        .await
                        .unwrap_or_default();
                    let resolver = crate::resolver_now(state).await;
                    let mut out = Vec::new();
                    for s in slugs {
                        if let Ok(Some(p)) = repos::posts::find_by_slug(&state.db, &s, kind, true).await {
                            let j = vuuut_core::models::post_to_json(&p, &[], &[], "");
                            out.push((p.title.unwrap_or_default(), resolver(&j)));
                        }
                    }
                    out
                }
            };
            let items: String = rows.iter().map(|(n, u)| format!(r#"<li><a href="{}">{}</a></li>"#, esc(u), esc(n))).collect();
            format!(
                r#"<div class="vb-sitemap" style="--vb-gc:{}">{}<ul>{items}</ul></div>"#,
                q["columns"].as_i64().unwrap_or(2),
                match st("title") { "" => String::new(), t => format!("<h3>{}</h3>", esc(t)) },
            )
        }
        "posts" => draw_posts(state, id, q, post, member).await,
        _ => String::new(),
    }
}

async fn draw_posts(state: &AppState, id: &str, q: &Value, post: Option<&Value>, member: &Value) -> String {
    let st = |k: &str| q[k].as_str().unwrap_or("");
    let flag = |k: &str| q[k].as_bool().unwrap_or(false);
    let count = q["count"].as_i64().unwrap_or(6).clamp(1, 48);
    let offset = q["offset"].as_i64().unwrap_or(0).clamp(0, 50);
    let more = st("pagination") == "more" || flag("filter");
    let mut filters: Vec<String> = Vec::new();
    match st("source") {
        "featured" => filters.push("featured:true".into()),
        "tag" if !st("tag").is_empty() => filters.push(format!("tag:{}", st("tag"))),
        "author" if !st("author").is_empty() => filters.push(format!("author:{}", st("author"))),
        "related" => {
            if let Some(t) = post.and_then(|p| p["primary_tag"]["slug"].as_str()) {
                filters.push(format!("tag:{t}"));
            }
        }
        _ => {}
    }
    let current = post.and_then(|p| p["id"].as_str());
    if let (true, Some(cur)) = (flag("exclude_current") || st("source") == "related", current) {
        filters.push(format!("id:-{cur}"));
    }
    let filter = if filters.is_empty() { None } else { vuuut_nql::parse(&filters.join("+")).ok() };
    let order = match st("order") {
        "oldest" => "published_at asc",
        "title" => "title asc",
        _ => "published_at desc",
    };
    let limit = (if more { count * 4 } else { count }) + offset;
    let kind = if st("kind") == "page" { "page" } else { "post" };
    let page = repos::posts::browse(
        &state.db,
        &repos::posts::BrowsePosts { filter, order: Some(order.into()), limit: limit.min(200), page: 1, post_type: Some(kind.into()), public: true, include_authors: true, include_tags: true, ..Default::default() },
    )
    .await;
    let posts: Vec<vuuut_core::models::Post> = page.map(|p| p.posts).unwrap_or_default().into_iter().skip(offset as usize).collect();
    let json = crate::routes::posts_page_json(state, &posts, member).await;
    if json.is_empty() {
        return r#"<p class="vb-posts-empty" style="opacity:.6">No posts to show yet.</p>"#.into();
    }
    let show_image = q["show_image"].as_bool().unwrap_or(true);
    let show_excerpt = q["show_excerpt"].as_bool().unwrap_or(true);
    let show_meta = q["show_meta"].as_bool().unwrap_or(true);
    let read_more = st("read_more");
    let mut all_tags: Vec<(String, String)> = Vec::new();
    let cards: String = json
        .iter()
        .map(|p| {
            let url = esc(p["url"].as_str().unwrap_or("#"));
            let tags: Vec<String> = p["tags"].as_array().into_iter().flatten().filter_map(|t| t["slug"].as_str().map(String::from)).collect();
            for t in p["tags"].as_array().into_iter().flatten() {
                if let (Some(s), Some(n)) = (t["slug"].as_str(), t["name"].as_str()) {
                    if !all_tags.iter().any(|(x, _)| x == s) && t["visibility"].as_str() != Some("internal") {
                        all_tags.push((s.into(), n.into()));
                    }
                }
            }
            let img = match (show_image, p["feature_image"].as_str()) {
                (true, Some(i)) => format!(r#"<a class="vb-pc-img" href="{url}"><img src="{}" alt="" loading="lazy"></a>"#, esc(i)),
                (true, None) => format!(r#"<a class="vb-pc-img" href="{url}" aria-hidden="true" tabindex="-1"></a>"#),
                _ => String::new(),
            };
            let tag = if flag("show_tags") { p["primary_tag"]["name"].as_str().map(|n| format!(r#"<span class="vb-pc-tag">{}</span>"#, esc(n))).unwrap_or_default() } else { String::new() };
            let excerpt = if show_excerpt { p["excerpt"].as_str().or(p["custom_excerpt"].as_str()).map(|e| format!("<p>{}</p>", esc(&e.chars().take(180).collect::<String>()))).unwrap_or_default() } else { String::new() };
            let meta = if show_meta {
                format!(
                    r#"<span class="vb-pc-meta">{}{}</span>"#,
                    esc(&fmt_date(p["published_at"].as_str().unwrap_or(""))),
                    p["primary_author"]["name"].as_str().map(|n| format!(" · {}", esc(n))).unwrap_or_default()
                )
            } else {
                String::new()
            };
            let more_link = if read_more.is_empty() { String::new() } else { format!(r#"<a class="vb-pc-more" href="{url}">{} →</a>"#, esc(read_more)) };
            format!(r#"<article class="vb-pc" data-tags="{}">{img}{tag}<h3><a href="{url}">{}</a></h3>{excerpt}{meta}{more_link}</article>"#, esc(&tags.join(" ")), esc(p["title"].as_str().unwrap_or("")))
        })
        .collect();
    let layout = match st("layout") { "" => "cards", l => l };
    let filter_bar = if flag("filter") && !all_tags.is_empty() {
        let b: String = all_tags.iter().map(|(s, n)| format!(r#"<button type="button" data-tag="{}">{}</button>"#, esc(s), esc(n))).collect();
        format!(r#"<div class="vb-filter"><button type="button" class="on" data-tag="*">All</button>{b}</div>"#)
    } else {
        String::new()
    };
    let more_btn = if st("pagination") == "more" { r#"<button type="button" class="vb-btn vb-btn-outline vb-btn-m vb-more" hidden>Load more</button>"# } else { "" };
    if layout == "carousel" {
        return format!(
            r#"<div class="vb-posts-wrap vb-pk-cards"><div class="vb-carousel" data-vb-carousel data-arrows="true" data-dots="true" data-autoplay="0" style="--vb-pv:{}"><div class="vb-track">{cards}</div></div></div>"#,
            q["columns"].as_i64().unwrap_or(3)
        );
    }
    let _ = id;
    format!(r#"<div class="vb-posts-wrap vb-pk-{layout}" data-count="{count}">{filter_bar}<div class="vb-posts">{cards}</div>{more_btn}</div>"#)
}

/// The request-time pass. `post` is the entry's JSON when there is one,
/// `original` its own body when a single-post template wraps it.
pub async fn expand(state: &AppState, html: &str, post: Option<&Value>, original: Option<&str>, member: &Value, path: &str) -> String {
    let facts = dynamic::Facts::now(member_state(member));
    let mut out = dynamic::apply_conditions(html, &facts);
    // dynamic widgets: collect, draw (async), then put back in order
    let mut wanted: Vec<(String, String, Value)> = Vec::new();
    dynamic::replace_dyn(&out, |k, id, q| {
        wanted.push((k.to_string(), id.to_string(), q.clone()));
        String::new()
    });
    if !wanted.is_empty() {
        let mut drawn = Vec::with_capacity(wanted.len());
        for (k, id, q) in &wanted {
            drawn.push(draw(state, k, id, q, post, original, member, path).await);
        }
        let mut it = drawn.into_iter();
        out = dynamic::replace_dyn(&out, |_, _, _| it.next().unwrap_or_default());
    }
    // a Post content widget's body can itself carry builder markers? no —
    // it is the post's own HTML, so tags are filled last and only once
    // Form widgets: the same placeholder the editor's Form card leaves
    out = vuuut_api::forms::expand_html(state, out).await;
    let title = state.settings.get_str("title").unwrap_or_default();
    let desc = state.settings.get_str("description").unwrap_or_default();
    let now = chrono::Utc::now();
    dynamic::fill_tags(&out, |name| {
        let p = post.unwrap_or(&Value::Null);
        Some(match name {
            "post.title" => p["title"].as_str()?.to_string(),
            "post.excerpt" => p["excerpt"].as_str().or(p["custom_excerpt"].as_str())?.to_string(),
            "post.date" => fmt_date(p["published_at"].as_str()?),
            "post.updated" => fmt_date(p["updated_at"].as_str()?),
            "post.reading_time" => format!("{} min read", p["reading_time"].as_i64().unwrap_or(1).max(1)),
            "post.author" => p["primary_author"]["name"].as_str()?.to_string(),
            "post.author_bio" => p["primary_author"]["bio"].as_str()?.to_string(),
            "post.tags" => p["tags"].as_array()?.iter().filter_map(|t| t["name"].as_str()).collect::<Vec<_>>().join(", "),
            "post.primary_tag" => p["primary_tag"]["name"].as_str()?.to_string(),
            "post.url" => p["url"].as_str()?.to_string(),
            "post.feature_image" => p["feature_image"].as_str()?.to_string(),
            "post.author_url" => p["primary_author"]["url"].as_str()?.to_string(),
            "post.author_image" => p["primary_author"]["profile_image"].as_str()?.to_string(),
            "post.primary_tag_url" => p["primary_tag"]["url"].as_str()?.to_string(),
            "site.title" => title.clone(),
            "site.description" => desc.clone(),
            "site.url" => state.site_url(),
            "site.logo" => state.settings.get_str("logo").map(|l| vuuut_core::url::base_asset(&l))?,
            "site.icon" => state.settings.get_str("icon").map(|l| vuuut_core::url::base_asset(&l))?,
            "site.cover" => state.settings.get_str("cover_image").map(|l| vuuut_core::url::base_asset(&l))?,
            "member.name" => member["firstname"].as_str().map(String::from).unwrap_or_default(),
            "year" => now.format("%Y").to_string(),
            "today" => now.format("%-d %b %Y").to_string(),
            _ => return None,
        })
    })
}

// ---------------------------------------------------------------------------
// Theme Builder parts on every page
// ---------------------------------------------------------------------------

/// System addresses no part is ever added to.
fn system_path(path: &str) -> bool {
    ["/ghost", "/members/", "/content/", "/public/", "/assets/", "/.well-known/", "/email/", "/r/", "/webmentions/"].iter().any(|p| path.starts_with(p))
        || path.ends_with(".xml")
        || path.ends_with(".txt")
        || path.ends_with(".json")
        || path.contains("/api/")
}

async fn read_body(resp: Response<Body>) -> Option<(axum::http::response::Parts, String)> {
    let (parts, body) = resp.into_parts();
    let bytes = axum::body::to_bytes(body, 20_000_000).await.ok()?;
    Some((parts, String::from_utf8(bytes.to_vec()).ok()?))
}

fn rebuild(mut parts: axum::http::response::Parts, body: String) -> Response<Body> {
    parts.headers.remove(header::CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(body))
}

/// Put the Theme Builder's header, footer and popups into an HTML page.
pub async fn site_parts(State(state): State<AppState>, req: axum::extract::Request, next: axum::middleware::Next) -> Response<Body> {
    let path = req.uri().path().to_string();
    let canvas = req.uri().query().is_some_and(|q| q.contains("vb=canvas"));
    let headers = req.headers().clone();
    let resp = next.run(req).await;
    let s = settings(&state);
    let html_type = resp.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|v| v.starts_with("text/html"));
    if !s.enabled || !html_type || system_path(&path) || resp.headers().contains_key(header::CONTENT_ENCODING) {
        return resp;
    }
    let pick = |kind: &'static str| {
        let state = state.clone();
        let path = path.clone();
        async move {
            repos::builder::active_parts(&state.db, kind)
                .await
                .into_iter()
                .map(|(id, html, o)| (id, html, PartOptions::from_value(&serde_json::from_str(&o).unwrap_or(json!({})))))
                .filter(|(_, _, o)| o.applies_to_path(&path))
                .collect::<Vec<_>>()
        }
    };
    let header_p = pick("header").await.into_iter().max_by_key(|p| p.2.priority);
    let footer_p = pick("footer").await.into_iter().max_by_key(|p| p.2.priority);
    let popups = if canvas { Vec::new() } else { pick("popup").await };
    if header_p.is_none() && footer_p.is_none() && popups.is_empty() {
        return resp;
    }
    let Some((parts, mut body)) = read_body(resp).await else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "").into_response();
    };
    let member = crate::render::member_context(&state, &headers).await;
    let wrap = |p: &(String, String, PartOptions), html: String| match p.2.cond() {
        Some(c) => dynamic::apply_conditions(&format!("{}{html}{}", dynamic::open_marker(&format!("part{}", p.0), &c), dynamic::close_marker(&format!("part{}", p.0))), &dynamic::Facts::now(member_state(&member))),
        None => html,
    };
    let mut top = String::new();
    let mut hide = Vec::new();
    if let Some(p) = &header_p {
        let html = expand(&state, &p.1, None, None, &member, &path).await;
        top = wrap(p, format!(r#"<div class="vb-part-header" data-vb-part="{}">{}{html}</div>"#, esc(&p.0), asset_tags()));
        if !top.is_empty() && !s.hide_header.is_empty() {
            hide.push(s.hide_header.clone());
        }
    }
    let mut bottom = String::new();
    if let Some(p) = &footer_p {
        let html = expand(&state, &p.1, None, None, &member, &path).await;
        let f = wrap(p, format!(r#"<div class="vb-part-footer" data-vb-part="{}">{html}</div>"#, esc(&p.0)));
        if !f.is_empty() && !s.hide_footer.is_empty() {
            hide.push(s.hide_footer.clone());
        }
        bottom.push_str(&f);
    }
    for p in &popups {
        let html = expand(&state, &p.1, None, None, &member, &path).await;
        bottom.push_str(&wrap(p, vuuut_core::builder::parts::popup_html(&p.0, &p.2, &html)));
    }
    if !bottom.is_empty() && top.is_empty() {
        bottom.insert_str(0, &asset_tags());
    }
    if !hide.is_empty() {
        // the theme's own parts, hidden — never ours
        top.insert_str(0, &format!("<style>:is({}):not(.vb-part-header *):not(.vb-part-footer *){{display:none!important}}</style>", hide.join(",")));
    }
    if let Some(i) = body.find("<body").and_then(|i| body[i..].find('>').map(|j| i + j + 1)) {
        body.insert_str(i, &top);
    }
    if let Some(i) = body.rfind("</body>") {
        body.insert_str(i, &bottom);
    }
    rebuild(parts, body)
}

// ---------------------------------------------------------------------------
// coming soon / maintenance
// ---------------------------------------------------------------------------

pub async fn maintenance(State(state): State<AppState>, req: axum::extract::Request, next: axum::middleware::Next) -> Response<Body> {
    let s = settings(&state);
    let path = req.uri().path().to_string();
    if s.maintenance == b::Maintenance::Off || s.maintenance_page.is_empty() || system_path(&path) || path.starts_with("/p/") || req.method() != axum::http::Method::GET {
        return next.run(req).await;
    }
    let headers: HeaderMap = req.headers().clone();
    if vuuut_api::admin::current_user(&state, &headers).await.is_some() {
        let mut resp = next.run(req).await;
        resp.headers_mut().insert("x-vuuut-maintenance", "staff-bypass".parse().unwrap());
        return resp;
    }
    let Some(post) = repos::posts::find_by_id(&state.db, &s.maintenance_page).await.ok().flatten() else {
        return next.run(req).await;
    };
    let member = crate::render::member_context(&state, &headers).await;
    let html = match repos::builder::active_html(&state.db, &post.id).await {
        Some(h) => format!("{}{}", asset_tags(), expand(&state, &h, None, None, &member, &path).await),
        None => post.html.clone().unwrap_or_default(),
    };
    let resp = crate::routes::render_virtual_page_hidden(&state, &headers, &path, post.title.as_deref().unwrap_or(""), html).await;
    let (mut parts, body) = resp.into_parts();
    parts.headers.insert("x-robots-tag", "noindex".parse().unwrap());
    parts.headers.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    if s.maintenance == b::Maintenance::Maintenance {
        parts.status = StatusCode::SERVICE_UNAVAILABLE;
        parts.headers.insert(header::RETRY_AFTER, "3600".parse().unwrap());
    }
    Response::from_parts(parts, body)
}
