//! Pulling remote images into local storage when a post is saved.

use super::*;

// ---------------------------------------------------------------------------
// media inliner
// ---------------------------------------------------------------------------

/// Rehost the remote images posts point at.
///
/// An imported post almost always still links to the site it came from, so
/// the pictures vanish the day that site does. This walks every post's body,
/// fetches each image hosted on one of `domains`, saves it under
/// `content/images/`, and rewrites the post to point at the local copy.
///
/// Scoped to named domains on purpose. Ghost's own endpoint is too, and for
/// the same reason: "inline the images from my old blog" is a migration step,
/// while "inline every image on the internet" is a way to fill a disk.
pub(crate) async fn inline_remote_media(
    state: &AppState,
    domains: &[String],
) -> Result<serde_json::Value, ghost_core::GhostError> {
    let posts: Vec<(String, Option<ghost_db::types::Text>, Option<ghost_db::types::Text>)> =
        sqlx::query_as("SELECT id, html, lexical FROM posts WHERE html IS NOT NULL OR lexical IS NOT NULL")
            .fetch_all(&state.db)
            .await
            .map_err(|e| ghost_core::GhostError::Db(e.to_string()))?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        // Every hop re-checked, not just the URL in the post: these addresses
        // come out of imported content, so they are a stranger's choice twice
        // over, and a plain redirect limit would let one reach this host's
        // own network.
        .redirect(ghost_api::mentions::guarded_redirect(3))
        .build()
        .unwrap_or_default();
    // One fetch per distinct URL however many posts carry it, and the map is
    // also what makes the rewrite consistent across html and lexical.
    let mut rehosted: HashMap<String, String> = HashMap::new();
    let mut failed = 0usize;
    let mut changed_posts = 0usize;

    for (id, html, lexical) in posts {
        let mut new_html = html.clone();
        let mut new_lexical = lexical.clone();
        let mut touched = false;
        let sources: Vec<String> = [html.as_deref(), lexical.as_deref()]
            .into_iter()
            .flatten()
            .flat_map(remote_image_urls)
            .collect();
        for url in sources {
            if !domains.iter().any(|domain| url_host_matches(&url, domain)) {
                continue;
            }
            let local = match rehosted.get(&url) {
                Some(local) => local.clone(),
                None => match fetch_and_store_image(state, &client, &url).await {
                    Ok(local) => {
                        rehosted.insert(url.clone(), local.clone());
                        local
                    }
                    Err(e) => {
                        tracing::debug!("media inliner: {url} could not be rehosted: {}", e.message());
                        failed += 1;
                        continue;
                    }
                },
            };
            if let Some(body) = new_html.as_mut() {
                if body.contains(&url) {
                    *body = ghost_db::types::Text(body.replace(&url, &local));
                    touched = true;
                }
            }
            if let Some(body) = new_lexical.as_mut() {
                // Lexical holds the URL inside a JSON string, so the escaped
                // form is what appears there.
                let escaped = url.replace('/', "\\/");
                if body.contains(&url) || body.contains(&escaped) {
                    *body = ghost_db::types::Text(
                        body.replace(&url, &local).replace(&escaped, &local.replace('/', "\\/")),
                    );
                    touched = true;
                }
            }
        }
        if touched {
            let _ = sqlx::query("UPDATE posts SET html = ?, lexical = ?, updated_at = ? WHERE id = ?")
                .bind(&new_html)
                .bind(&new_lexical)
                .bind(ghost_core::dates::now_db())
                .bind(&id)
                .execute(&state.db)
                .await;
            changed_posts += 1;
        }
    }
    Ok(serde_json::json!({
        "inlined": rehosted.len(),
        "posts_updated": changed_posts,
        "failed": failed,
    }))
}

/// Absolute `http(s)` image URLs in a document.
pub(crate) fn remote_image_urls(body: &str) -> Vec<String> {
    const EXTENSIONS: &[&str] = &[".jpg", ".jpeg", ".png", ".gif", ".webp", ".avif", ".svg"];
    let mut found = Vec::new();
    for marker in ["http://", "https://"] {
        let mut from = 0;
        while let Some(at) = body[from..].find(marker) {
            let start = from + at;
            // A URL ends at the first character that cannot be in one.
            let end = body[start..]
                .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | ')' | '\\'))
                .map(|e| start + e)
                .unwrap_or(body.len());
            let url = &body[start..end];
            let lower = url.to_ascii_lowercase();
            let path = lower.split(['?', '#']).next().unwrap_or(&lower);
            if EXTENSIONS.iter().any(|ext| path.ends_with(ext)) {
                found.push(url.to_string());
            }
            from = end.max(start + marker.len());
        }
    }
    found.sort();
    found.dedup();
    found
}

/// Whether a URL's host is, or is a subdomain of, `domain`.
pub(crate) fn url_host_matches(url: &str, domain: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else { return false };
    let Some(host) = parsed.host_str().map(str::to_ascii_lowercase) else { return false };
    let domain = domain.trim_start_matches("www.");
    let host = host.trim_start_matches("www.");
    host == domain || host.ends_with(&format!(".{domain}"))
}

/// Fetch one image and save it where an upload would have gone.
pub(crate) async fn fetch_and_store_image(
    state: &AppState,
    client: &reqwest::Client,
    url: &str,
) -> Result<String, ghost_core::GhostError> {
    // The same guard the webmention receiver uses: this fetches a URL that
    // came out of post content, which an import put there.
    let Some(parsed) = ghost_api::mentions::fetchable(url) else {
        return Err(ghost_core::GhostError::BadRequest("not a fetchable URL".into()));
    };
    let response = client
        .get(parsed)
        .send()
        .await
        .map_err(|e| ghost_core::GhostError::BadRequest(e.to_string()))?;
    if !response.status().is_success() {
        return Err(ghost_core::GhostError::BadRequest(format!("returned {}", response.status())));
    }
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let extension = match content_type.as_str() {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/avif" => "avif",
        "image/svg+xml" => "svg",
        other => return Err(ghost_core::GhostError::BadRequest(format!("unsupported type {other}"))),
    };
    const MAX_BYTES: usize = 25 * 1024 * 1024;
    let bytes = response.bytes().await.map_err(|e| ghost_core::GhostError::BadRequest(e.to_string()))?;
    if bytes.len() > MAX_BYTES {
        return Err(ghost_core::GhostError::BadRequest("larger than the upload limit".into()));
    }

    // Same layout as an upload: `content/images/YYYY/MM/name.ext`.
    let stamp = ghost_core::dates::now_db();
    let (year, month) = (&stamp[..4], &stamp[5..7]);
    let dir = state.config.images_dir().join(year).join(month);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| ghost_core::GhostError::Internal(format!("could not create the image directory: {e}")))?;
    let stem: String = url
        .rsplit('/')
        .next()
        .unwrap_or("image")
        .split(['?', '#'])
        .next()
        .unwrap_or("image")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(60)
        .collect();
    let stem = if stem.is_empty() { "image".to_string() } else { stem };
    // Suffixed with a short random tail, so two sources with the same file
    // name do not overwrite one another.
    let name = format!("{stem}-{}.{extension}", ghost_core::ids::random_hex(4));
    let path = dir.join(&name);
    tokio::fs::write(&path, &bytes)
        .await
        .map_err(|e| ghost_core::GhostError::Internal(format!("could not write the image: {e}")))?;
    let local = format!("/content/images/{year}/{month}/{name}");
    // Through the storage adapter, so an S3-backed install gets the same
    // treatment as a local one.
    ghost_api::storage::publish_upload(
        state,
        &format!("{year}/{month}/{name}"),
        &content_type,
        &bytes,
        local,
    )
    .await
}
