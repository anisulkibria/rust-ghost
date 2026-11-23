//! Sitemap index + per-resource sitemaps, in two representations.
//!
//! ## Why there are two
//!
//! A sitemap is XML because crawlers read it, and raw XML is what a browser
//! shows the person who follows the link from robots.txt or from Search
//! Console: an unstyled, unclickable tree.
//!
//! The usual fix is an `<?xml-stylesheet?>` instruction pointing at an XSLT
//! transform. That is not usable any more: Chrome has deprecated XSLT and
//! paints a red "this functionality is being removed — contact the maintainer
//! of the site" banner above every transformed document, which is worse than
//! the raw XML it was meant to improve.
//!
//! So the same two listings are rendered server-side instead, and which one a
//! request gets is decided by [`wants_html`].
//!
//! ## The rule, and why it is safe
//!
//! HTML is served **only** to a genuine top-level browser navigation — all
//! three of `Sec-Fetch-Mode: navigate`, `Sec-Fetch-Dest: document` and an
//! `Accept` that asks for `text/html`. Everything else gets the XML, byte for
//! byte what it got before this existed.
//!
//! The `Sec-Fetch-*` pair is what makes this sound rather than a guess at user
//! agents. They are forbidden header names: a page's JavaScript cannot set
//! them, and a plain HTTP client does not send them at all — they are written
//! by the browser itself, and only `document`/`navigate` for a URL typed,
//! clicked or bookmarked. A sitemap fetcher is not a navigation.
//!
//! The default is the safe one. Any request this cannot positively identify as
//! a person looking at a page in a browser is served XML, so the failure mode
//! of an unfamiliar client is "it works", not "the sitemap stopped parsing".
//! `Vary` is set so a cache never hands one representation to the other
//! audience.
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use vuuut_api::AppState;
use vuuut_db::repos;

/// The caches that may hold a sitemap have to key on what decided it.
pub(crate) const VARY: &str = "Accept, Sec-Fetch-Dest, Sec-Fetch-Mode";

/// Whether this request is a person opening the URL in a browser.
///
/// All three signals are required, and the answer is `false` unless every one
/// of them is present and says so — see the module docs for why the default
/// has to fall on the XML side.
pub(crate) fn wants_html(headers: &HeaderMap) -> bool {
    let value = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.trim().to_ascii_lowercase())
    };
    // Set by the browser on a top-level navigation, and unforgeable from a
    // page: both are forbidden header names.
    if value("sec-fetch-mode").as_deref() != Some("navigate") {
        return false;
    }
    if value("sec-fetch-dest").as_deref() != Some("document") {
        return false;
    }
    // A browser navigation always asks for HTML first. A client that sends the
    // Sec-Fetch pair but wants XML — a crawler that grew navigation semantics,
    // say — is taken at its word.
    value("accept").is_some_and(|accept| accept.contains("text/html"))
}

/// One address in a sitemap.
///
/// `title` is never written to the XML — a sitemap has nowhere to put one —
/// and exists because the HTML rendering can say "Our 2026 roadmap" where the
/// XML can only say `/our-2026-roadmap/`. Rendering server-side rather than in
/// a stylesheet is what makes it available at all.
struct Entry {
    loc: String,
    lastmod: Option<String>,
    title: Option<String>,
    /// The page's own image, for the `<image:image>` block Google Images
    /// reads. Absolute, because a sitemap has no base to resolve against.
    image: Option<String>,
}

/// One sub-sitemap, as listed by the index.
struct IndexEntry {
    loc: String,
    /// `posts` — the resource, which is also its label once capitalised.
    resource: &'static str,
    /// The newest `lastmod` of anything inside it, so a crawler can skip a
    /// sub-sitemap that has not changed since it last read it. Without this
    /// the index says only "these four exist" and every pass refetches all
    /// four.
    lastmod: Option<String>,
}

const RESOURCES: [&str; 4] = ["posts", "pages", "tags", "authors"];

pub(crate) fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// `posts` -> `Posts`.
fn label(resource: &str) -> String {
    let mut chars = resource.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// `https://example.com/a/b` -> `/a/b`, and the site root -> `/`.
fn path_of(url: &str) -> &str {
    let rest = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    match rest.find('/') {
        Some(at) => &rest[at..],
        None => "/",
    }
}

/// `https://example.com/a/b` -> `example.com`.
pub(crate) fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    rest.split('/').next().unwrap_or(rest)
}

/// `1 address` / `2 addresses` / `4 sitemaps`.
///
/// Appending a bare `s` is not enough for the words this module actually uses:
/// "address" is one of them, and "addresss" is the kind of thing nobody
/// notices until it is on a public page.
fn countable(n: usize, singular: &str) -> String {
    if n == 1 {
        return format!("1 {singular}");
    }
    let sibilant = singular.ends_with('s')
        || singular.ends_with('x')
        || singular.ends_with('z')
        || singular.ends_with("ch")
        || singular.ends_with("sh");
    format!("{n} {singular}{}", if sibilant { "es" } else { "s" })
}

// ---------------------------------------------------------------------------
// the XML, unchanged by any of the above
// ---------------------------------------------------------------------------

fn index_xml(entries: &[IndexEntry]) -> String {
    let body: String = entries
        .iter()
        .map(|e| {
            let lastmod = e
                .lastmod
                .as_deref()
                .map(|when| format!("<lastmod>{}</lastmod>", esc(when)))
                .unwrap_or_default();
            format!("<sitemap><loc>{}</loc>{lastmod}</sitemap>\n", esc(&e.loc))
        })
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<sitemapindex xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
{body}
</sitemapindex>"#,
    )
}

/// The filename an image URL ends in, which is all a sitemap caption has to
/// go on — Ghost uses `path.basename` here for the same reason.
fn image_caption(url: &str) -> &str {
    let without_query = url.split(['?', '#']).next().unwrap_or(url);
    without_query.rsplit('/').next().unwrap_or(without_query)
}

fn urlset_xml(entries: &[Entry]) -> String {
    let body: String = entries
        .iter()
        .map(|e| {
            let lastmod = e
                .lastmod
                .as_deref()
                .map(|when| format!("<lastmod>{}</lastmod>", esc(when)))
                .unwrap_or_default();
            // Declared in the urlset element below whether or not any entry
            // has one: a namespace prefix used nowhere is valid, and deciding
            // per document would mean two different root elements to test.
            let image = e
                .image
                .as_deref()
                .map(|src| {
                    format!(
                        "<image:image><image:loc>{}</image:loc><image:caption>{}</image:caption></image:image>",
                        esc(src),
                        esc(image_caption(src))
                    )
                })
                .unwrap_or_default();
            format!("<url><loc>{}</loc>{lastmod}{image}</url>\n", esc(&e.loc))
        })
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:image="http://www.google.com/schemas/sitemap-image/1.1">
{body}
</urlset>"#,
    )
}

/// A response with a `HeaderMap`, since the header set differs per branch and
/// axum's tuple form needs a fixed-size array.
fn respond(content_type: &'static str, extra: &[(header::HeaderName, &'static str)], body: String) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, header::HeaderValue::from_static(content_type));
    // Set on both representations: a cache that saw one must not serve it to
    // the audience that would have got the other.
    headers.insert(header::VARY, header::HeaderValue::from_static(VARY));
    for (name, value) in extra {
        headers.insert(name.clone(), header::HeaderValue::from_static(value));
    }
    (StatusCode::OK, headers, body).into_response()
}

fn xml_response(xml: String, cache: Option<&'static str>) -> Response {
    match cache {
        Some(cache) => respond("text/xml; charset=utf-8", &[(header::CACHE_CONTROL, cache)], xml),
        None => respond("text/xml; charset=utf-8", &[], xml),
    }
}

fn html_response(html: String) -> Response {
    respond(
        "text/html; charset=utf-8",
        // The canonical thing at this URL is the XML. The page is a
        // convenience for whoever opened it, not something to index.
        &[(header::HeaderName::from_static("x-robots-tag"), "noindex")],
        html,
    )
}

// ---------------------------------------------------------------------------
// the page
// ---------------------------------------------------------------------------

/// Self-contained on purpose: no webfont, no stylesheet, no image. This page
/// is reached from a crawler's report or a robots.txt link, often on a slow
/// connection, and it should be useful before anything else loads — there is
/// nothing else to load.
pub(crate) const CSS: &str = r#"
:root {
  color-scheme: light dark;
  --bg: #fbfbfc; --surface: #fff; --text: #15151a; --muted: #6c6c78;
  --line: #e7e7ec; --brand: #c9135a; --brand-tint: #fff2f7;
  --shadow: 0 1px 2px rgba(18,18,26,.04), 0 8px 24px rgba(18,18,26,.06);
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #111114; --surface: #1a1a1f; --text: #ededf2; --muted: #9b9ba6;
    --line: #2b2b33; --brand: #ff7fab; --brand-tint: #26151d; --shadow: none;
  }
}
* { box-sizing: border-box; }
html { -webkit-text-size-adjust: 100%; }
body { margin: 0; background: var(--bg); color: var(--text);
  font: 15px/1.6 -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif;
  -webkit-font-smoothing: antialiased; -moz-osx-font-smoothing: grayscale; }
main { max-width: 920px; margin: 0 auto; padding: 56px 24px 96px; }
.eyebrow { display: inline-block; margin: 0 0 14px; padding: 4px 11px; border-radius: 999px;
  background: var(--brand-tint); color: var(--brand); font-size: 12px; font-weight: 600;
  letter-spacing: .04em; text-transform: uppercase; }
h1 { margin: 0 0 10px; font-size: clamp(28px, 5vw, 40px); line-height: 1.15; font-weight: 700;
  letter-spacing: -.02em; word-break: break-word; }
.lead { margin: 0 0 36px; max-width: 62ch; color: var(--muted); font-size: 16px; }
.lead strong { color: var(--text); font-weight: 600; }
.cards { display: grid; grid-template-columns: repeat(auto-fill, minmax(210px, 1fr)); gap: 14px; }
.card { display: flex; flex-direction: column; gap: 6px; padding: 20px; background: var(--surface);
  border: 1px solid var(--line); border-radius: 16px; box-shadow: var(--shadow); color: inherit;
  text-decoration: none; transition: border-color .15s ease, transform .15s ease; }
.card:hover, .card:focus-visible { border-color: var(--brand); transform: translateY(-2px); }
.card:focus-visible { outline: 2px solid var(--brand); outline-offset: 3px; }
.card-name { font-size: 17px; font-weight: 600; letter-spacing: -.01em; }
.card-count { color: var(--muted); font-size: 13px; }
.card-go { margin-top: 4px; color: var(--brand); font-size: 13px; font-weight: 600; }
.panel { background: var(--surface); border: 1px solid var(--line); border-radius: 16px;
  box-shadow: var(--shadow); overflow: hidden; }
table { width: 100%; border-collapse: collapse; }
th { padding: 13px 20px; background: var(--bg); border-bottom: 1px solid var(--line);
  color: var(--muted); font-size: 11px; font-weight: 600; letter-spacing: .07em;
  text-transform: uppercase; text-align: left; }
td { padding: 12px 20px; border-top: 1px solid var(--line); vertical-align: baseline; }
tbody tr:first-child td { border-top: 0; }
tbody tr:hover { background: var(--brand-tint); }
.num { width: 1%; color: var(--muted); font-size: 13px; font-variant-numeric: tabular-nums;
  text-align: right; white-space: nowrap; }
.addr a { color: var(--brand); text-decoration: none; font-weight: 500; }
.addr a:hover { text-decoration: underline; }
.addr a:focus-visible { outline: 2px solid var(--brand); outline-offset: 2px; border-radius: 3px; }
.addr .slug { display: block; margin-top: 2px; color: var(--muted); font-weight: 400;
  font-family: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace;
  font-size: 12.5px; word-break: break-all; }
.when { width: 1%; color: var(--muted); font-size: 13px; white-space: nowrap;
  font-variant-numeric: tabular-nums; }
.back { margin: 22px 0 0; font-size: 14px; }
.back a { color: var(--brand); font-weight: 600; text-decoration: none; }
.back a:hover { text-decoration: underline; }
.note { margin: 40px 0 0; padding-top: 24px; border-top: 1px solid var(--line);
  color: var(--muted); font-size: 13.5px; max-width: 68ch; }
.note code { padding: 1px 5px; border-radius: 5px; background: var(--brand-tint);
  color: var(--brand); font-family: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace;
  font-size: 12.5px; }
.empty { padding: 40px 20px; text-align: center; color: var(--muted); }
@media (max-width: 560px) {
  main { padding: 36px 16px 64px; }
  th, td { padding-left: 14px; padding-right: 14px; }
  .num { display: none; }
}
"#;

fn shell(title: &str, body: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="robots" content="noindex">
<title>{title}</title>
<style>{CSS}</style>
</head>
<body><main>{body}</main></body>
</html>"#,
        title = esc(title),
    )
}

fn index_html(site_url: &str, entries: &[IndexEntry], counts: &[(&str, usize)]) -> String {
    let cards: String = entries
        .iter()
        .map(|e| {
            let count = counts
                .iter()
                .find(|(r, _)| *r == e.resource)
                .map(|(_, n)| countable(*n, "address"))
                .unwrap_or_default();
            format!(
                r#"<a class="card" href="{href}"><span class="card-name">{name}</span>
<span class="card-count">{count}</span><span class="card-go">Open</span></a>"#,
                href = esc(&e.loc),
                name = esc(&label(e.resource)),
                count = esc(&count),
            )
        })
        .collect();
    let body = format!(
        r#"<p class="eyebrow">Sitemap</p>
<h1>{host}</h1>
<p class="lead">This site's addresses are listed across <strong>{n}</strong>. Open one to see the pages in it.</p>
<div class="cards">{cards}</div>
<p class="note">A sitemap tells search engines which pages exist on this site and when they last
changed. This is the index: each entry above is a sitemap of its own. The same addresses are served
as XML to anything that is not a browser, so a crawler reading <code>/sitemap.xml</code> sees exactly
what it expects.</p>"#,
        host = esc(host_of(site_url)),
        n = esc(&countable(entries.len(), "sitemap")),
        cards = cards,
    );
    shell(&format!("Sitemap — {}", host_of(site_url)), &body)
}

fn urlset_html(site_url: &str, resource: &str, entries: &[Entry]) -> String {
    let dated = entries.iter().any(|e| e.lastmod.as_deref().is_some_and(|d| !d.is_empty()));
    let rows: String = entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let path = path_of(&e.loc);
            // The title leads where there is one, because it is what tells one
            // page from another; the path sits under it, and the full address
            // is the href. A page with no title is its path.
            let shown = match e.title.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
                Some(title) => format!(
                    r#"<a href="{href}">{title}</a><span class="slug">{path}</span>"#,
                    href = esc(&e.loc),
                    title = esc(title),
                    path = esc(path),
                ),
                None => format!(
                    r#"<a href="{href}"><span class="slug" style="margin:0">{path}</span></a>"#,
                    href = esc(&e.loc),
                    path = esc(path),
                ),
            };
            let when = if dated {
                let value = e
                    .lastmod
                    .as_deref()
                    .filter(|d| !d.is_empty())
                    .map(|d| d.chars().take(10).collect::<String>())
                    .unwrap_or_else(|| "—".into());
                format!(r#"<td class="when">{}</td>"#, esc(&value))
            } else {
                String::new()
            };
            format!(
                r#"<tr><td class="num">{n}</td><td class="addr">{shown}</td>{when}</tr>"#,
                n = i + 1,
            )
        })
        .collect();

    let table = if entries.is_empty() {
        r#"<div class="panel"><p class="empty">Nothing is published in this sitemap yet.</p></div>"#.to_string()
    } else {
        format!(
            r#"<div class="panel"><table>
<thead><tr><th class="num">#</th><th>Page</th>{when_head}</tr></thead>
<tbody>{rows}</tbody></table></div>"#,
            when_head = if dated { r#"<th class="when">Last changed</th>"# } else { "" },
        )
    };
    let lead = if entries.is_empty() {
        String::new()
    } else {
        format!(
            r#"<p class="lead"><strong>{n}</strong> in this sitemap{tail}</p>"#,
            n = esc(&countable(entries.len(), "address")),
            tail = if entries.len() == 1 { ", and it is a link." } else { ". Every one of them is a link." },
        )
    };
    let body = format!(
        r#"<p class="eyebrow">Sitemap · {label}</p>
<h1>{host}</h1>
{lead}{table}
<p class="back"><a href="{root}">← All sitemaps</a></p>
<p class="note">These are the addresses search engines are told about.{dates} The same list is
served as XML to anything that is not a browser.</p>"#,
        label = esc(&label(resource)),
        host = esc(host_of(site_url)),
        // Built from the site URL rather than written relative, so it is right
        // under a base path as well as at a domain root.
        root = esc(&format!("{}/sitemap.xml", site_url.trim_end_matches('/'))),
        dates = if dated { " The dates are when each page was last edited." } else { "" },
    );
    shell(&format!("{} sitemap — {}", label(resource), host_of(site_url)), &body)
}

// ---------------------------------------------------------------------------
// handlers
// ---------------------------------------------------------------------------

pub async fn sitemap_index(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let site_url = state.site_url();
    let root = site_url.trim_end_matches('/');
    // Both representations need the contents now: the XML for each
    // sub-sitemap's `lastmod`, the page for its counts.
    let mut entries: Vec<IndexEntry> = Vec::new();
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for resource in RESOURCES {
        let listed = collect(&state, resource).await.unwrap_or_default();
        counts.push((resource, listed.len()));
        entries.push(IndexEntry {
            loc: format!("{root}/sitemap-{resource}.xml"),
            resource,
            lastmod: listed.iter().filter_map(|e| e.lastmod.clone()).max(),
        });
    }

    if !wants_html(&headers) {
        return xml_response(index_xml(&entries), None);
    }
    html_response(index_html(&site_url, &entries, &counts))
}

pub async fn sitemap_resource(state: AppState, headers: HeaderMap, resource: String) -> Response {
    let Some(entries) = collect(&state, &resource).await else {
        return (StatusCode::NOT_FOUND, "unknown sitemap").into_response();
    };
    if wants_html(&headers) {
        return html_response(urlset_html(&state.site_url(), &resource, &entries));
    }
    xml_response(urlset_xml(&entries), Some("public, max-age=3600"))
}

/// Every address in one sub-sitemap, or `None` if there is no such sitemap.
async fn collect(state: &AppState, resource: &str) -> Option<Vec<Entry>> {
    let site_url = state.site_url();
    let resolver = crate::resolver_now(state).await;
    let absolute = |url: &str| vuuut_core::url::absolute(&site_url, url);
    let w3c = "%Y-%m-%dT%H:%M:%S%:z";
    let when = |stored: Option<&str>| {
        stored
            .and_then(vuuut_core::dates::from_db_string)
            .map(|d| d.format(w3c).to_string())
    };
    // Settings → SEO can turn the image blocks off. They are on by default
    // because they are how a site's pictures get into image search at all.
    let images = vuuut_core::seo::flag(
        &serde_json::json!({ "seo_sitemap_images": state.settings.get("seo_sitemap_images") }),
        "seo_sitemap_images",
    );
    let image = |path: Option<&String>| {
        images
            .then(|| path.filter(|p| !p.is_empty()))
            .flatten()
            .map(|p| absolute(&vuuut_core::url::base_asset(p)))
    };

    match resource {
        "posts" | "pages" => {
            let post_type = resource.trim_end_matches('s');
            let posts = repos::posts::all_published(&state.db).await.unwrap_or_default();
            Some(
                posts
                    .iter()
                    .filter(|p| p.post_type == post_type)
                    .map(|p| {
                        let j = serde_json::json!({ "slug": p.slug, "type": p.post_type, "status": "published" });
                        Entry {
                            loc: absolute(&resolver(&j)),
                            lastmod: when(p.updated_at.as_deref()),
                            title: p.title.clone(),
                            image: image(p.feature_image.as_ref()),
                        }
                    })
                    .collect(),
            )
        }
        "tags" => {
            let tags = repos::tags::all_public(&state.db).await.unwrap_or_default();
            Some(
                tags.iter()
                    .map(|t| Entry {
                        loc: absolute(&resolver(&vuuut_core::models::tag_to_json(t, None))),
                        lastmod: when(t.updated_at.as_deref()),
                        title: t.name.clone(),
                        image: image(t.feature_image.as_ref()),
                    })
                    .collect(),
            )
        }
        "authors" => {
            let users = repos::users::list(&state.db, true).await.unwrap_or_default();
            Some(
                users
                    .iter()
                    .map(|u| Entry {
                        loc: absolute(&resolver(&vuuut_core::models::user_public_json(u, None))),
                        lastmod: when(u.updated_at.as_deref()),
                        title: u.name.clone(),
                        // Ghost prefers the cover here: it is the one a person
                        // chose for their archive page.
                        image: image(u.cover_image.as_ref().or(u.profile_image.as_ref())),
                    })
                    .collect(),
            )
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                axum::http::HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    const BROWSER: &[(&str, &str)] = &[
        ("sec-fetch-mode", "navigate"),
        ("sec-fetch-dest", "document"),
        ("accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"),
    ];

    /// The whole arrangement rests on this: anything that is not positively a
    /// browser navigation gets the XML. A false positive would hand a crawler
    /// an HTML document where it expects a sitemap.
    #[test]
    fn only_a_real_browser_navigation_is_offered_the_page() {
        assert!(wants_html(&headers(BROWSER)));
        // Case is not significant in header values here.
        assert!(wants_html(&headers(&[
            ("sec-fetch-mode", "Navigate"),
            ("sec-fetch-dest", "Document"),
            ("accept", "TEXT/HTML"),
        ])));

        // A bare fetcher: curl, Googlebot, a feed reader.
        assert!(!wants_html(&HeaderMap::new()));
        assert!(!wants_html(&headers(&[("accept", "*/*")])));
        // Googlebot's Accept names text/html, which is exactly why Accept
        // alone must never be enough to decide this.
        assert!(!wants_html(&headers(&[(
            "accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
        )])));

        // Every one of the three is load-bearing: drop any and it is XML.
        for omit in ["sec-fetch-mode", "sec-fetch-dest", "accept"] {
            let kept: Vec<(&str, &str)> =
                BROWSER.iter().copied().filter(|(k, _)| *k != omit).collect();
            assert!(!wants_html(&headers(&kept)), "{omit} was not required");
        }

        // A browser fetching the URL as a subresource is not someone reading
        // it: an <img>, a fetch(), a prefetch.
        assert!(!wants_html(&headers(&[
            ("sec-fetch-mode", "cors"),
            ("sec-fetch-dest", "empty"),
            ("accept", "text/html"),
        ])));
        assert!(!wants_html(&headers(&[
            ("sec-fetch-mode", "navigate"),
            ("sec-fetch-dest", "iframe"),
            ("accept", "text/html"),
        ])));
        // Navigating but asking for XML is taken at its word.
        assert!(!wants_html(&headers(&[
            ("sec-fetch-mode", "navigate"),
            ("sec-fetch-dest", "document"),
            ("accept", "application/xml"),
        ])));
    }

    fn entry(loc: &str, title: Option<&str>, lastmod: Option<&str>) -> Entry {
        Entry {
            loc: loc.into(),
            lastmod: lastmod.map(str::to_string),
            title: title.map(str::to_string),
            image: None,
        }
    }

    #[test]
    fn the_xml_is_what_it_always_was() {
        let entries = vec![
            entry("https://example.test/a/", Some("A post"), Some("2026-11-14T10:00:00+00:00")),
            entry("https://example.test/b/", None, None),
        ];
        let xml = urlset_xml(&entries);
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(xml.contains(r#"<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:image="http://www.google.com/schemas/sitemap-image/1.1">"#));
        assert!(xml.contains("<url><loc>https://example.test/a/</loc><lastmod>2026-11-14T10:00:00+00:00</lastmod></url>"));
        // No lastmod rather than an empty one, and never a title: a sitemap
        // has nowhere to put one and a crawler would reject the element.
        assert!(xml.contains("<url><loc>https://example.test/b/</loc></url>"));
        assert!(!xml.contains("A post"), "the title must not reach the XML");
        assert!(!xml.contains("<?xml-stylesheet"), "the XSLT instruction is gone");
        assert!(xml.trim_end().ends_with("</urlset>"));
    }

    #[test]
    fn the_page_links_every_address_and_leads_with_its_title() {
        let entries = vec![
            entry("https://example.test/our-roadmap/", Some("Our 2026 roadmap"), Some("2026-11-14T10:00:00+00:00")),
            entry("https://example.test/untitled/", None, None),
        ];
        let html = urlset_html("https://example.test", "posts", &entries);
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains(r#"<a href="https://example.test/our-roadmap/">Our 2026 roadmap</a>"#), "{html}");
        // The path is shown under the title, and is the whole link when there
        // is no title.
        assert!(html.contains(r#"<span class="slug">/our-roadmap/</span>"#));
        assert!(html.contains(r#"<a href="https://example.test/untitled/"><span class="slug" style="margin:0">/untitled/</span></a>"#));
        assert!(html.contains("2026-11-14"), "the date column is missing");
        assert!(html.contains("Last changed"));
        assert!(html.contains("2 addresses"));
        assert!(html.contains(r#"<a href="https://example.test/sitemap.xml">"#), "no way back to the index");
    }

    #[test]
    fn a_sitemap_with_no_dates_does_not_draw_an_empty_column() {
        let entries = vec![entry("https://example.test/tag/news/", Some("News"), None)];
        let html = urlset_html("https://example.test", "tags", &entries);
        assert!(!html.contains("Last changed"), "an empty column reads as missing data");
        assert!(!html.contains("The dates are when"), "it promises dates it has none of");
        assert!(html.contains("1 address") && html.contains("and it is a link."));
        assert!(html.contains("Sitemap · Tags"));
    }

    #[test]
    fn an_empty_sitemap_says_so_instead_of_counting_to_zero() {
        let html = urlset_html("https://example.test", "pages", &[]);
        assert!(html.contains("Nothing is published in this sitemap yet."));
        assert!(!html.contains("0 addresses"), "a sentence about nothing");
        assert!(!html.contains("<table"), "an empty table has no headings to show");
    }

    #[test]
    fn a_title_from_the_database_is_escaped_before_it_reaches_the_page() {
        let hostile = r#"<img src=x onerror=alert(1)>"#;
        let entries = vec![entry("https://example.test/x/?a=1&b=2", Some(hostile), None)];
        let html = urlset_html("https://example.test", "posts", &entries);
        assert!(!html.contains("<img src=x"), "{html}");
        assert!(html.contains("&lt;img src=x"));
        // …and so is the address it is linked to.
        assert!(html.contains("https://example.test/x/?a=1&amp;b=2"));
        // The same value in the XML is escaped too.
        assert!(urlset_xml(&entries).contains("?a=1&amp;b=2"));
    }

    #[test]
    fn the_index_lists_every_resource_in_both_representations() {
        let entries: Vec<IndexEntry> = RESOURCES
            .iter()
            .map(|r| IndexEntry {
                loc: format!("https://example.test/sitemap-{r}.xml"),
                resource: r,
                lastmod: Some("2026-11-14T10:00:00+00:00".into()),
            })
            .collect();
        let xml = index_xml(&entries);
        assert!(xml.contains(r#"<sitemapindex xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">"#));
        assert!(!xml.contains("<urlset"), "an index is not a urlset");
        for r in RESOURCES {
            assert!(xml.contains(&format!("<loc>https://example.test/sitemap-{r}.xml</loc>")));
        }

        let counts = [("posts", 10), ("pages", 1), ("tags", 0), ("authors", 3)];
        let html = index_html("https://example.test", &entries, &counts);
        assert!(html.contains("4 sitemaps"));
        for (r, n) in counts {
            assert!(html.contains(&format!(r#"href="https://example.test/sitemap-{r}.xml""#)));
            assert!(html.contains(&countable(n, "address")), "{r} has no count");
        }
        assert!(html.contains("Posts") && html.contains("Authors"));
    }

    #[test]
    fn urls_are_split_into_their_parts_without_a_url_parser() {
        assert_eq!(path_of("https://example.test/a/b/"), "/a/b/");
        assert_eq!(path_of("https://example.test/"), "/");
        assert_eq!(path_of("https://example.test"), "/");
        assert_eq!(path_of("https://example.test/blog/p/?q=1"), "/blog/p/?q=1");
        assert_eq!(host_of("https://example.test/a/b/"), "example.test");
        assert_eq!(host_of("http://localhost:1971/x"), "localhost:1971");
        assert_eq!(host_of("https://example.test"), "example.test");
        assert_eq!(label("posts"), "Posts");
        assert_eq!(label(""), "");
        assert_eq!(countable(1, "address"), "1 address");
        assert_eq!(countable(0, "address"), "0 addresses");
        assert_eq!(countable(2, "address"), "2 addresses");
        assert_eq!(countable(4, "sitemap"), "4 sitemaps");
        assert_eq!(countable(1, "sitemap"), "1 sitemap");
    }
}
