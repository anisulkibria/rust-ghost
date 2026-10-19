//! Sitemap index + per-resource sitemaps.
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use ghost_api::AppState;
use ghost_db::repos;

pub async fn sitemap_index(State(state): State<AppState>) -> Response {
    let site_url = state.site_url();
    let resources = ["posts", "pages", "tags", "authors"];
    let mut entries = String::new();
    for r in resources {
        entries.push_str(&format!(
            "<sitemap><loc>{}/sitemap-{}.xml</loc></sitemap>\n",
            site_url.trim_end_matches('/'),
            r
        ));
    }
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:image="http://www.google.com/schemas/sitemap-image/1.1">
{entries}
</urlset>"#,
    );
    // sitemap index uses <sitemapindex>
    let xml = xml.replace(
        "<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\" xmlns:image=\"http://www.google.com/schemas/sitemap-image/1.1\">",
        "<sitemapindex xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">",
    );
    let xml = xml.replace("</urlset>", "</sitemapindex>");
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/xml")], xml).into_response()
}

pub async fn sitemap_resource(state: AppState, resource: String) -> Response {
    let site_url = state.site_url();
    let resolver = crate::resolver_now(&state).await;
    let mut urls = String::new();
    let w3c = "%Y-%m-%dT%H:%M:%S%:z";

    match resource.as_str() {
        "posts" | "pages" => {
            let post_type = resource.trim_end_matches('s').to_string();
            let posts = repos::posts::all_published(&state.db).await.unwrap_or_default();
            for p in posts.iter().filter(|p| p.post_type == post_type) {
                let j = serde_json::json!({ "slug": p.slug, "type": p.post_type, "status": "published" });
                let url = resolver(&j);
                let updated = p
                    .updated_at
                    .as_deref()
                    .and_then(ghost_core::dates::from_db_string)
                    .map(|d| d.format(w3c).to_string())
                    .unwrap_or_default();
                urls.push_str(&format!(
                    "<url><loc>{}{}</loc><lastmod>{}</lastmod></url>\n",
                    site_url.trim_end_matches('/'),
                    url,
                    updated
                ));
            }
        }
        "tags" => {
            let tags = repos::tags::all_public(&state.db).await.unwrap_or_default();
            for t in &tags {
                let j = ghost_core::models::tag_to_json(t, None);
                let url = resolver(&j);
                urls.push_str(&format!(
                    "<url><loc>{}{}</loc></url>\n",
                    site_url.trim_end_matches('/'),
                    url
                ));
            }
        }
        "authors" => {
            let users = repos::users::list(&state.db, true).await.unwrap_or_default();
            for u in &users {
                let j = ghost_core::models::user_public_json(u, None);
                let url = resolver(&j);
                urls.push_str(&format!(
                    "<url><loc>{}{}</loc></url>\n",
                    site_url.trim_end_matches('/'),
                    url
                ));
            }
        }
        _ => return (StatusCode::NOT_FOUND, "unknown sitemap").into_response(),
    }

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
{urls}
</urlset>"#,
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/xml"), (header::CACHE_CONTROL, "public, max-age=3600")],
        xml,
    )
        .into_response()
}
