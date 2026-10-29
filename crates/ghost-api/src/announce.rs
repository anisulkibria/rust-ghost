//! Telling the rest of the web that something was published.
//!
//! Two protocols, both fire-and-forget, both off by default: a publish must
//! not wait on a third party, and a site that has not asked to be announced
//! should not be reaching out to one.
//!
//! * **IndexNow** — one POST tells Bing, Yandex and the other participating
//!   engines that a URL changed, instead of waiting for a crawl. Ghost has
//!   the `indexnow_enabled` setting; this is the ping behind it.
//! * **Webmentions** — handled in [`crate::mentions`], called from here so
//!   that a publish has one place that announces it.
use crate::AppState;

/// The file an IndexNow key has to be served from, which is how an engine
/// confirms the key belongs to whoever is using it.
pub fn key_file_path(key: &str) -> String {
    format!("/{key}.txt")
}

/// This site's IndexNow key, if it has one worth using.
///
/// Length-checked because the key doubles as a path: a one-character key
/// would make `/a.txt` a reserved URL, and IndexNow asks for 8–128
/// hexadecimal-ish characters anyway.
pub fn indexnow_key(state: &AppState) -> Option<String> {
    state
        .settings
        .get_str("indexnow_api_key")
        .map(|key| key.trim().to_string())
        .filter(|key| (8..=128).contains(&key.len()) && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
}

/// Announce a published post.
///
/// Called with the post's own URL and its rendered body: the URL for the
/// search engines, the body for the sites it links to.
pub fn published(state: &AppState, post_url: String, html: String) {
    index_now(state, post_url.clone());
    crate::mentions::send_for_post(state, post_url, html);
}

/// Submit a URL to IndexNow.
///
/// The key is a site-generated secret that also has to be readable at
/// `/{key}.txt`; the frontend serves that file from the same setting. An
/// engine fetches it to confirm whoever submitted the URL controls the site,
/// which is why one without the other does nothing.
pub fn index_now(state: &AppState, url: String) {
    let Some(key) = indexnow_key(state) else {
        return;
    };
    let Ok(parsed) = reqwest::Url::parse(&url) else { return };
    let Some(host) = parsed.host_str().map(str::to_owned) else { return };
    // A site nobody outside can reach has nothing to tell a search engine.
    if host == "localhost" || host.ends_with(".localhost") || host.parse::<std::net::IpAddr>().is_ok() {
        tracing::debug!("indexnow: {host} is not a public host, so nothing was submitted");
        return;
    }
    tokio::spawn(async move {
        let endpoint = format!(
            "https://api.indexnow.org/indexnow?url={}&key={}",
            percent_encoding::utf8_percent_encode(&url, percent_encoding::NON_ALPHANUMERIC),
            percent_encoding::utf8_percent_encode(&key, percent_encoding::NON_ALPHANUMERIC),
        );
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        match client.get(&endpoint).send().await {
            // 200 and 202 both mean accepted; 422 means the key did not
            // validate, which is worth saying out loud because it is a
            // configuration problem rather than a transient one.
            Ok(response) if response.status().is_success() => {
                tracing::debug!("indexnow: submitted {url}");
            }
            Ok(response) if response.status().as_u16() == 422 => {
                tracing::warn!(
                    "indexnow: {url} was refused — the key must also be readable at /{key}.txt on this site"
                );
            }
            Ok(response) => tracing::debug!("indexnow: {url} returned {}", response.status()),
            Err(e) => tracing::debug!("indexnow: {url} could not be submitted: {e}"),
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_key_file_is_the_key_at_the_site_root() {
        assert_eq!(super::key_file_path("abc123def456"), "/abc123def456.txt");
    }
}
