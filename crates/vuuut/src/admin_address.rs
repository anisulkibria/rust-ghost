use axum::{
    body::{self, Body},
    extract::Request,
    http::{header, uri::PathAndQuery, Uri, StatusCode},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};
use vuuut_api::AppState;
use vuuut_core::{admin_address as settings, url};

pub async fn route(state: &AppState, mut request: Request, next: Next) -> Response {
    let current = format!("/{}", state.admin_slug());
    let path = request.uri().path();
    let query = request.uri().query().map(|query| format!("?{query}")).unwrap_or_default();

    if let Some(tail) = settings::suffix(path, &current) {
        if tail.is_empty() {
            return temporary(&url::base(&format!("{current}/{query}")));
        }
        let mut parts = request.uri().clone().into_parts();
        let Ok(path) = format!("/ghost{tail}{query}").parse::<PathAndQuery>() else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        parts.path_and_query = Some(path);
        let Ok(uri) = Uri::from_parts(parts) else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        *request.uri_mut() = uri;

        let response = next.run(request).await;
        // Saving the address can change the route used by this very response.
        return public_response(response, state).await;
    }

    if legacy_content(path) {
        // Ghost themes and the upstream Portal and Search scripts use this
        // public API address. It never dispatches to an admin handler.
        return next.run(request).await;
    }
    let alias = settings::suffix(path, "/vuuut").or_else(|| settings::suffix(path, "/ghost"));
    if let Some(tail) = alias {
        let redirect = state.settings.get(settings::REDIRECT_KEY).is_none()
            || state.settings.get_bool(settings::REDIRECT_KEY);
        if redirect {
            let tail = if tail.is_empty() { "/" } else { tail };
            return temporary(&url::base(&format!("{current}{tail}{query}")));
        }
        let mut response = StatusCode::NOT_FOUND.into_response();
        response.headers_mut().insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
        return response;
    }
    next.run(request).await
}

fn temporary(target: &str) -> Response {
    let mut response = Redirect::temporary(target).into_response();
    response.headers_mut().insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
}

fn legacy_content(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("/ghost/api/") else { return false };
    let mut segments = rest.split('/');
    match segments.next() {
        Some("content") => true,
        Some("v2" | "v3" | "v4" | "v5" | "v6") => segments.next() == Some("content"),
        _ => false,
    }
}

async fn public_response(mut response: Response, state: &AppState) -> Response {
    let target = state.admin_path();
    let subdir = url::subdir();
    let internal = format!("{subdir}/ghost");
    for name in [header::LOCATION, header::CONTENT_LOCATION] {
        let location = response.headers().get(&name).and_then(|v| v.to_str().ok());
        let replacement = location
            .and_then(|location| settings::suffix(location, &internal).or_else(|| settings::suffix(location, "/ghost")))
            .and_then(|tail| format!("{target}{tail}").parse().ok());
        if let Some(value) = replacement {
            response.headers_mut().insert(name, value);
        }
    }

    let rewrite = response.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok())
        .is_some_and(|kind| kind.starts_with("text/html") || kind.starts_with("text/css") || kind.contains("javascript"));
    if !rewrite || response.headers().contains_key(header::CONTENT_ENCODING) {
        return response;
    }
    let (mut parts, data) = response.into_parts();
    let Ok(bytes) = body::to_bytes(data, usize::MAX).await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Response::from_parts(parts, Body::from(bytes));
    };
    let public_segment = url::strip(&target);
    let Some(rewritten) = rewrite_text(text, subdir, public_segment) else {
        return Response::from_parts(parts, Body::from(bytes));
    };
    parts.headers.remove(header::CONTENT_LENGTH);
    parts.headers.remove(header::ETAG);
    Response::from_parts(parts, Body::from(rewritten))
}

/// Admin markup and scripts use root-relative URL literals. Match at both
/// boundaries so links to Ghost on another domain and asset names stay intact.
fn rewrite_text(text: &str, subdir: &str, target: &str) -> Option<String> {
    let mut output = None::<String>;
    let mut copied = 0;
    for (index, _) in text.match_indices("/ghost") {
        let before = &text[..index];
        let based = !subdir.is_empty() && before.ends_with(subdir);
        let start = if based { index - subdir.len() } else { index };
        let end = index + "/ghost".len();
        let left = text[..start].chars().next_back();
        let right = text[end..].chars().next();
        let left_ok = left.is_none_or(|ch| matches!(ch, '"' | '\'' | '`' | '(' | '=' | ' '));
        let right_ok = right.is_none_or(|ch| matches!(ch, '/' | '?' | '#' | '"' | '\'' | '`' | ')' | ' '));
        if left_ok && right_ok {
            let output = output.get_or_insert_with(|| String::with_capacity(text.len()));
            output.push_str(&text[copied..start]);
            if based {
                output.push_str(subdir);
            }
            output.push_str(target);
            copied = end;
        }
    }
    output.map(|mut output| {
        output.push_str(&text[copied..]);
        output
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_admin_links_without_rewriting_external_links_or_names() {
        let text = r#"<a href="/blog/ghost/posts/">Posts</a><script>fetch('/ghost/api/admin/');</script><a href="https://example.test/ghost/">Other</a><img src="/ghost-logo.png">"#;
        let output = rewrite_text(text, "/blog", "/staff").unwrap();
        assert!(output.contains(r#"href="/blog/staff/posts/""#));
        assert!(output.contains("fetch('/staff/api/admin/')"));
        assert!(output.contains("https://example.test/ghost/"));
        assert!(output.contains(r#"src="/ghost-logo.png""#));
    }

    #[test]
    fn public_api_compatibility_only_covers_content() {
        assert!(legacy_content("/ghost/api/content/posts/"));
        assert!(legacy_content("/ghost/api/v5/content/posts/"));
        assert!(!legacy_content("/ghost/api/admin/"));
        assert!(!legacy_content("/ghost/api/content-edit/"));
    }
}
