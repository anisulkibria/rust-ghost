//! RSS 2.0 feeds, in two representations.
//!
//! A feed is XML because feed readers read it, and raw XML is what a browser
//! shows the person who clicks the feed link: an unstyled tree with no way
//! to tell what to do with it. So, exactly as the sitemap does (see
//! `crate::sitemap` for the rule and why it is safe), a genuine top-level
//! browser navigation gets a page — what the feed is, its address ready to
//! copy into a reader, and the posts in it — and every other request gets
//! the XML byte for byte. A feed reader never sends the `Sec-Fetch` pair
//! that decides it, so the failure mode of an unfamiliar client is "it gets
//! the feed".
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use ghost_api::AppState;
use ghost_db::repos;
use ghost_routing::tree::RssKind;

/// `locale` is the language the feed's URL names: `/bn/rss/` is the Bengali
/// feed and carries only Bengali posts, as `/bn/` lists only them. On a
/// single-language site it changes nothing.
pub async fn render_rss(state: AppState, kind: RssKind, name: String, path: &str, locale: &str, headers: &HeaderMap) -> Response {
    let site_url = state.site_url();
    let locales = crate::routes::site_locales(&state);
    let site_title = state.settings.get_str("title").unwrap_or_else(|| "Ghost".into());
    let site_desc = state.settings.get_str("description").unwrap_or_default();

    // which posts?
    let (title, filter): (String, Option<ghost_nql::Expr>) = match kind {
        RssKind::Collection => (site_title.clone(), None),
        RssKind::Tag => (
            format!("{} - Tag: {name}", site_title),
            ghost_nql::parse(&format!("tags:'{name}'")).ok(),
        ),
        RssKind::Author => (
            format!("{} - Author: {name}", site_title),
            ghost_nql::parse(&format!("authors:'{name}'")).ok(),
        ),
    };

    let q = repos::posts::BrowsePosts {
        filter,
        order: None,
        limit: 15,
        page: 1,
        post_type: Some("post".into()),
        public: true,
        include_authors: true,
        include_tags: true,
        locale: locales.multilingual().then(|| locale.to_string()),
        default_locale: Some(locales.default_locale().to_string()),
    };
    let result = match repos::posts::browse(&state.db, &q).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("rss: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, "rss error").into_response();
        }
    };

    let resolver = crate::resolver_now(&state).await;
    let mut items = String::new();
    // The same posts, for the page a browser gets.
    let mut cards: Vec<Card> = Vec::new();
    for p in &result.posts {
        let rel = repos::posts::load_relations(&state.db, std::slice::from_ref(p))
            .await
            .unwrap_or_default();
        let authors = rel.authors_for(&p.id);
        let tags = rel.tags_for(&p.id);
        let mut j = ghost_core::models::post_to_json(p, &authors, &tags, "");
        j["url"] = serde_json::json!(resolver(&j));
        ghost_core::models::apply_post_access(&mut j, None, &[]);
        let url = j.get("url").and_then(|u| u.as_str()).unwrap_or("");
        let title = j.get("title").and_then(|t| t.as_str()).unwrap_or("");
        let desc = j.get("excerpt").and_then(|t| t.as_str()).unwrap_or("");
        let pub_date = j
            .get("published_at")
            .and_then(|t| t.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.format("%a, %d %b %Y %H:%M:%S %z").to_string())
            .unwrap_or_default();
        // `url` already carries the base path (the resolver adds it), so the
        // absolute form has to be built against the origin, not against a
        // site url that carries it as well
        let abs = ghost_core::url::absolute(&site_url, url);
        cards.push(Card {
            title: title.to_string(),
            url: abs.clone(),
            excerpt: desc.to_string(),
            date: j
                .get("published_at")
                .and_then(|t| t.as_str())
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.format("%-d %b %Y").to_string())
                .unwrap_or_default(),
            image: j
                .get("feature_image")
                .and_then(|i| i.as_str())
                .filter(|i| !i.is_empty())
                .map(|i| ghost_core::url::absolute(&site_url, i)),
        });
        items.push_str(&format!(
            "<item>\n<title>{}</title>\n<description>{}</description>\n<link>{}</link>\n<guid isPermaLink=\"true\">{}</guid>\n<pubDate>{}</pubDate>\n</item>\n",
            xml_escape(title),
            xml_escape(desc),
            xml_escape(&abs),
            xml_escape(&abs),
            pub_date,
        ));
    }

    let feed_url = format!("{}{}", site_url.trim_end_matches('/'), path);
    let home_url = format!("{}{}", site_url.trim_end_matches('/'), locales.localise("/", locale));

    if crate::sitemap::wants_html(headers) {
        // The feeds of the other languages, for a multilingual site: the
        // same kind of feed, under each language's prefix.
        let neutral = locales.split_path(path).1;
        let others: Vec<(String, String)> = if locales.multilingual() {
            locales
                .enabled()
                .iter()
                .map(|code| {
                    (
                        ghost_core::locales::native_name(code),
                        format!("{}{}", site_url.trim_end_matches('/'), locales.localise(&neutral, code)),
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        let page = feed_page(&FeedPage {
            title: &title,
            description: &site_desc,
            feed_url: &feed_url,
            home_url: &home_url,
            lang: &locales.resolve(Some(locale)),
            languages: &others,
            cards: &cards,
        });
        let mut out = HeaderMap::new();
        out.insert(header::CONTENT_TYPE, header::HeaderValue::from_static("text/html; charset=utf-8"));
        out.insert(header::VARY, header::HeaderValue::from_static(crate::sitemap::VARY));
        // The feed is the canonical thing at this URL; the page is a
        // convenience for whoever opened it.
        out.insert(header::HeaderName::from_static("x-robots-tag"), header::HeaderValue::from_static("noindex"));
        return (StatusCode::OK, out, page).into_response();
    }
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:content="http://purl.org/rss/1.0/modules/content/" xmlns:atom="http://www.w3.org/2005/Atom">
<channel>
<title>{}</title>
<description>{}</description>
<link>{}</link>
<atom:link href="{}" rel="self" type="application/rss+xml" />
<language>{}</language>
<generator>Vuuut</generator>
<lastBuildDate>{}</lastBuildDate>
{}
</channel>
</rss>"#,
        xml_escape(&title),
        xml_escape(&site_desc),
        // The feed's own language home, not the default's.
        xml_escape(&home_url),
        xml_escape(&feed_url),
        xml_escape(&locales.resolve(Some(locale))),
        chrono::Utc::now().format("%a, %d %b %Y %H:%M:%S %z"),
        items,
    );

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/rss+xml; charset=UTF-8"),
            // A cache that saw the page must not hand it to a feed reader.
            (header::VARY, crate::sitemap::VARY),
        ],
        xml,
    )
        .into_response()
}

/// One post, as the browser page shows it.
struct Card {
    title: String,
    url: String,
    excerpt: String,
    date: String,
    image: Option<String>,
}

struct FeedPage<'a> {
    title: &'a str,
    description: &'a str,
    feed_url: &'a str,
    home_url: &'a str,
    lang: &'a str,
    /// (language name, that language's feed URL), on a multilingual site.
    languages: &'a [(String, String)],
    cards: &'a [Card],
}

/// The feed's own styles, on top of the sitemap page's — the two are the
/// same family of page and should read as one.
const FEED_CSS: &str = r#"
.subscribe { display: flex; gap: 10px; align-items: stretch; margin: 0 0 14px; max-width: 640px; }
.subscribe input { flex: 1; min-width: 0; padding: 11px 14px; border: 1px solid var(--line); border-radius: 12px;
  background: var(--surface); color: var(--text); font: 13.5px ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace; }
.subscribe input:focus { outline: 2px solid var(--brand); outline-offset: 1px; }
.subscribe button { flex: none; padding: 0 18px; border: 0; border-radius: 12px; background: var(--brand); color: #fff;
  font: 600 14px/1 inherit; font-family: inherit; cursor: pointer; }
.subscribe button:focus-visible { outline: 2px solid var(--brand); outline-offset: 3px; }
.how { margin: 0 0 30px; color: var(--muted); font-size: 13.5px; max-width: 68ch; }
.langs { display: flex; flex-wrap: wrap; gap: 8px; margin: 0 0 30px; }
.langs a { padding: 6px 13px; border: 1px solid var(--line); border-radius: 999px; background: var(--surface);
  color: var(--text); font-size: 13.5px; font-weight: 500; text-decoration: none; }
.langs a:hover { border-color: var(--brand); color: var(--brand); }
.langs a[aria-current="true"] { border-color: var(--brand); background: var(--brand-tint); color: var(--brand); font-weight: 600; }
.posts { display: grid; gap: 14px; }
.post { display: grid; grid-template-columns: 1fr auto; gap: 18px; align-items: start; padding: 20px;
  background: var(--surface); border: 1px solid var(--line); border-radius: 16px; box-shadow: var(--shadow);
  color: inherit; text-decoration: none; transition: border-color .15s ease, transform .15s ease; }
.post:hover, .post:focus-visible { border-color: var(--brand); transform: translateY(-2px); }
.post:focus-visible { outline: 2px solid var(--brand); outline-offset: 3px; }
.post-date { margin: 0 0 4px; color: var(--muted); font-size: 12.5px; font-variant-numeric: tabular-nums; }
.post-title { margin: 0 0 6px; font-size: 18px; line-height: 1.3; font-weight: 650; letter-spacing: -.01em; }
.post-excerpt { margin: 0; color: var(--muted); font-size: 14.5px; display: -webkit-box; -webkit-line-clamp: 3;
  -webkit-box-orient: vertical; overflow: hidden; }
.post img { width: 120px; height: 84px; object-fit: cover; border-radius: 10px; background: var(--line); }
.section-title { margin: 0 0 14px; font-size: 13px; font-weight: 600; letter-spacing: .06em; text-transform: uppercase; color: var(--muted); }
@media (max-width: 560px) {
  .subscribe { flex-direction: column; }
  .subscribe button { padding: 12px; }
  .post { grid-template-columns: 1fr; }
  .post img { width: 100%; height: 160px; order: -1; }
}
"#;

/// The page a browser gets at a feed's address.
///
/// Self-contained like the sitemap page: no webfont, no external
/// stylesheet. The one script copies the address, and the page works
/// without it — the address is selectable text in a field.
fn feed_page(p: &FeedPage) -> String {
    use crate::sitemap::esc;
    let langs = if p.languages.len() > 1 {
        let links: String = p
            .languages
            .iter()
            .map(|(name, url)| {
                format!(
                    r#"<a href="{url}"{cur}>{name}</a>"#,
                    url = esc(url),
                    cur = if url == p.feed_url { r#" aria-current="true""# } else { "" },
                    name = esc(name),
                )
            })
            .collect();
        format!(r#"<p class="section-title">Languages</p><nav class="langs" aria-label="This feed in each language">{links}</nav>"#)
    } else {
        String::new()
    };
    let posts = if p.cards.is_empty() {
        r#"<div class="panel"><p class="empty">Nothing has been published here yet.</p></div>"#.to_string()
    } else {
        let list: String = p
            .cards
            .iter()
            .map(|c| {
                format!(
                    r#"<a class="post" href="{url}"><div>{date}<h2 class="post-title">{title}</h2>{excerpt}</div>{image}</a>"#,
                    url = esc(&c.url),
                    date = if c.date.is_empty() { String::new() } else { format!(r#"<p class="post-date">{}</p>"#, esc(&c.date)) },
                    title = esc(if c.title.trim().is_empty() { "(untitled)" } else { c.title.trim() }),
                    excerpt = if c.excerpt.trim().is_empty() {
                        String::new()
                    } else {
                        format!(r#"<p class="post-excerpt">{}</p>"#, esc(c.excerpt.trim()))
                    },
                    image = c
                        .image
                        .as_deref()
                        .map(|i| format!(r#"<img src="{}" alt="" loading="lazy">"#, esc(i)))
                        .unwrap_or_default(),
                )
            })
            .collect();
        format!(r#"<p class="section-title">Latest posts</p><div class="posts">{list}</div>"#)
    };
    let lead = if p.description.trim().is_empty() {
        format!(r#"The latest posts from <a href="{home}" style="color:var(--brand);font-weight:600;text-decoration:none">{host}</a>, as a feed."#,
            home = esc(p.home_url), host = esc(crate::sitemap::host_of(p.home_url)))
    } else {
        esc(p.description.trim())
    };
    format!(
        r#"<!doctype html>
<html lang="{lang}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="robots" content="noindex">
<title>{title} — RSS feed</title>
<link rel="alternate" type="application/rss+xml" title="{title}" href="{feed}">
<style>{css}{feed_css}</style>
</head>
<body><main>
<p class="eyebrow">RSS feed</p>
<h1>{title}</h1>
<p class="lead">{lead}</p>
<form class="subscribe" onsubmit="return false">
<input id="feed-url" value="{feed}" readonly aria-label="Feed address" onclick="this.select()">
<button type="button" id="copy">Copy address</button>
</form>
<p class="how">Paste this address into any feed reader to get new posts as soon as they are published. Feed readers read this page as RSS; it only looks like this in a browser.</p>
{langs}
{posts}
<p class="back"><a href="{home}">← Back to the site</a></p>
</main>
<script>
(function () {{
  var b = document.getElementById('copy'), i = document.getElementById('feed-url');
  if (!b || !i) return;
  b.addEventListener('click', function () {{
    var done = function () {{ b.textContent = 'Copied'; setTimeout(function () {{ b.textContent = 'Copy address'; }}, 1800); }};
    if (navigator.clipboard) {{ navigator.clipboard.writeText(i.value).then(done, function () {{ i.select(); }}); }}
    else {{ i.select(); document.execCommand('copy'); done(); }}
  }});
}})();
</script>
</body>
</html>"#,
        lang = esc(p.lang),
        title = esc(p.title),
        feed = esc(p.feed_url),
        css = crate::sitemap::CSS,
        feed_css = FEED_CSS,
        lead = lead,
        langs = langs,
        posts = posts,
        home = esc(p.home_url),
    )
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
