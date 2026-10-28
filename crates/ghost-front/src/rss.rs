//! RSS 2.0 feeds.
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use ghost_api::AppState;
use ghost_db::repos;
use ghost_routing::tree::RssKind;

pub async fn render_rss(state: AppState, kind: RssKind, name: String, path: &str) -> Response {
    let site_url = state.site_url();
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
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:content="http://purl.org/rss/1.0/modules/content/" xmlns:atom="http://www.w3.org/2005/Atom">
<channel>
<title>{}</title>
<description>{}</description>
<link>{}</link>
<atom:link href="{}" rel="self" type="application/rss+xml" />
<generator>Ghost-rust</generator>
<lastBuildDate>{}</lastBuildDate>
{}
</channel>
</rss>"#,
        xml_escape(&title),
        xml_escape(&site_desc),
        xml_escape(&site_url),
        xml_escape(&feed_url),
        chrono::Utc::now().format("%a, %d %b %Y %H:%M:%S %z"),
        items,
    );

    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "application/rss+xml; charset=UTF-8",
        )],
        xml,
    )
        .into_response()
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
