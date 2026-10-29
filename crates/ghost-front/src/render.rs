//! The render step: build the full template data (with @site/@config),
/// pick the first existing template, render inside spawn_blocking.
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use ghost_api::AppState;
use ghost_core::Result;
use ghost_render::{context as render_context, ThemeState};
use serde_json::{Map, Value};

pub struct RenderCtx {
    pub context: Vec<String>,
    pub relative_url: String,
    /// `@member` — the signed-in member, or `Null` for an anonymous visitor.
    /// Themes branch on this to swap "Sign in" for "Account", so it has to be
    /// resolved server-side even though Portal also fetches it over XHR.
    pub member: Value,
}

/// Resolve `@member` from the request's session cookie.
pub async fn member_context(state: &AppState, headers: &HeaderMap) -> Value {
    match ghost_api::members::current_member(state, headers).await {
        Some(m) => {
            let tiers = ghost_db::repos::tiers::member_tier_ids(&state.db, &m.id).await.unwrap_or_default();
            serde_json::json!({
            "uuid": m.uuid,
            "tiers": tiers,
            "email": m.email,
            "name": m.name,
            "firstname": m.name.as_deref().and_then(|n| n.split_whitespace().next()),
            "expertise": m.expertise,
            "status": m.status,
            "paid": m.status != "free",
            "created_at": m.created_at,
        })
        },
        None => Value::Null,
    }
}

/// The `status` of the `@member` context, for the access checks below.
pub fn member_status(member: &Value) -> Option<&str> {
    member.get("status").and_then(|v| v.as_str())
}

/// Recompute a serialized post's `access` flag for the visitor looking at it.
///
/// `post_to_json` can only answer the anonymous case — it has no request to
/// consult — so it sets `access` from `visibility` alone, which reads as
/// "members-only posts are closed to everybody". That is right for a
/// signed-out visitor and wrong for the member who paid for it.
///
/// `access` is not cosmetic: `{{content}}` swaps the post body for the
/// upgrade CTA when it is false, and `{{comments}}` renders nothing at all.
/// So without this a signed-in member sees a paywall on a post they have
/// access to, and no comment form under it.
pub fn member_tiers(member: &Value) -> Vec<String> {
    member.get("tiers").and_then(Value::as_array).map(|tiers| tiers.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

pub fn apply_member_tier_access(post: &mut Value, member_status: Option<&str>, member_tiers: &[String]) {
    ghost_core::models::apply_post_access(post, member_status, member_tiers);
}

/// Whether `{{comments}}` would render anything, as a root-level flag.
///
/// Themes wrap the comments section in `{{#if comments}}` — Source does
/// exactly that — so that a post with comments off does not get an empty
/// heading and a stray `<div>`. In handlebars.js that guard *calls* the
/// helper, because a registered helper wins an ambiguous reference;
/// handlebars-rust only consults the context, so the guard would always be
/// false and the section would never appear.
///
/// Seeding the flag fixes that, and it has to agree with the helper exactly:
/// if the flag were the looser of the two, a theme would render the wrapper
/// around nothing.
fn comments_available(data: &Value, site: &Value, frontend_key: &str) -> bool {
    if frontend_key.is_empty() {
        return false;
    }
    if matches!(site.get("comments_enabled").and_then(|v| v.as_str()), None | Some("off")) {
        return false;
    }
    let post = data.get("post").filter(|p| p.is_object());
    let Some(post) = post else { return false };
    // `comment_id` is the marker for "this is a post, not a page", and
    // `access` for "this visitor may read it".
    let is_post = post
        .get("comment_id")
        .and_then(|v| v.as_str())
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    is_post && post.get("access") != Some(&Value::Bool(false))
}

/// Render the first template in `candidates` that the theme has.
pub async fn render_template(
    state: &AppState,
    theme: &std::sync::Arc<ThemeState>,
    candidates: &[String],
    mut data: Value,
    rctx: &RenderCtx,
) -> Result<String> {
    // pick template
    let template = candidates
        .iter()
        .find(|c| theme.has_template(c))
        .or_else(|| theme.has_template("index").then(|| &candidates[candidates.len() - 1]))
        .cloned()
        .or_else(|| candidates.iter().find(|c| theme.has_template(c)).cloned());
    let Some(template) = template else {
        return Err(ghost_core::GhostError::ThemeError(format!(
            "no template found among {candidates:?}"
        )));
    };

    // build the full root: data + site/config/member/labs/custom + context
    let site = render_context::build_site(
        &state.settings,
        &state.site_url(),
        &state.admin_url(),
    );
    let config = render_context::build_config(
        theme.posts_per_page(),
        serde_json::to_value(&theme.pkg.config.image_sizes).unwrap_or(Value::Array(vec![])),
        theme.pkg.config.card_assets.any(),
        crate::cards::version(),
    );

    let mut config=config;
    if state.config.storage["active"].as_str().is_some_and(|s|matches!(s,"s3"|"S3")) {
        let s3=&state.config.storage["s3"];
        let prefix=s3["assetHost"].as_str().map(str::to_owned).unwrap_or_else(||{
            let endpoint=s3["endpoint"].as_str().map(str::to_owned).unwrap_or_else(||format!("https://s3.{}.amazonaws.com",s3["region"].as_str().unwrap_or("us-east-1")));
            format!("{}/{}",endpoint.trim_end_matches('/'),s3["bucket"].as_str().unwrap_or(""))
        });
        config["storage_image_prefix"]=serde_json::json!(format!("{}/images/",prefix.trim_end_matches('/')));
    }

    // `@custom`: the theme's declared settings at their defaults, with any
    // the site owner has changed applied on top
    let overrides = state
        .settings
        .get("custom_theme_settings")
        .and_then(|v| v.get(&theme.name).cloned());
    let custom = theme.custom_settings(overrides.as_ref());

    // `ghost_head` reads this to stamp `data-key` onto the Portal and Sodo
    // Search script tags; both are useless without it.
    let frontend_key = state.frontend_key().await.unwrap_or_default();

    // The flag goes on the post as well as the root: themes write the guard
    // inside a `{{#post}}` block, where a bare `comments` resolves against
    // the post rather than the root, and handlebars-rust does not walk up to
    // find it. Source's `post.hbs` is exactly that shape.
    let has_comments = comments_available(&data, &site, &frontend_key);
    if has_comments {
        for key in ["post", "page"] {
            if let Some(Value::Object(p)) = data.get_mut(key) {
                p.insert("comments".into(), Value::Bool(true));
            }
        }
    }

    // `@labs`: the flags set in Settings → Advanced. Themes guard beta markup
    // with `{{#if labs.someFlag}}`, so this has to be the stored object rather
    // than an empty one, or every such guard silently fails closed.
    let labs = match state.settings.get("labs") {
        Some(v @ Value::Object(_)) => v,
        // Rows written as a JSON *string* read the same as a real object.
        Some(Value::String(s)) => serde_json::from_str(&s).unwrap_or_else(|_| serde_json::json!({})),
        _ => serde_json::json!({}),
    };

    let root = match &mut data {
        Value::Object(m) => {
            m.insert("site".into(), site);
            m.insert("config".into(), config);
            m.insert("member".into(), rctx.member.clone());
            m.insert("labs".into(), labs.clone());
            m.insert("custom".into(), custom.clone());
            m.insert("context".into(), serde_json::json!(rctx.context));
            m.insert("relativeUrl".into(), Value::String(rctx.relative_url.clone()));
            m.insert("frontendKey".into(), Value::String(frontend_key.clone()));
            m.insert("comments".into(), Value::Bool(has_comments));
            Value::Object(m.clone())
        }
        other => {
            let mut m = Map::new();
            m.insert("this".into(), other.clone());
            m.insert("site".into(), site);
            m.insert("config".into(), config);
            m.insert("member".into(), rctx.member.clone());
            m.insert("labs".into(), labs);
            m.insert("custom".into(), custom.clone());
            m.insert("context".into(), serde_json::json!(rctx.context));
            m.insert("relativeUrl".into(), Value::String(rctx.relative_url.clone()));
            m.insert("frontendKey".into(), Value::String(frontend_key));
            m.insert("comments".into(), Value::Bool(has_comments));
            Value::Object(m)
        }
    };

    let theme = theme.clone();
    let template_name = template;
    let html = tokio::task::spawn_blocking(move || theme.render(&template_name, &root))
        .await
        .map_err(|e| ghost_core::GhostError::Internal(e.to_string()))??;
    Ok(html)
}

/// Render the theme's error page (error-<code> → error-<N>xx → error → builtin).
pub async fn error_page(
    state: &AppState,
    status: StatusCode,
    path: &str,
    headers: HeaderMap,
) -> Response {
    let code = status.as_u16();
    let theme = state.theme.read().await.clone();
    let nxx = format!("error-{}xx", code / 100);
    let candidates = vec![format!("error-{code}"), nxx, "error".to_string()];

    let message = if code == 404 {
        "Page not found".to_string()
    } else {
        status.canonical_reason().unwrap_or("Error").to_string()
    };

    let mut root = Map::new();
    root.insert("statusCode".into(), serde_json::json!(code));
    root.insert("message".into(), Value::String(message));
    root.insert("errorDetails".into(), Value::Null);
    root.insert("path".into(), Value::String(path.to_string()));

    let rctx = RenderCtx {
        context: vec![format!("error-{code}")],
        relative_url: path.to_string(),
        member: member_context(state, &headers).await,
    };

    if let Ok(html) = render_template(state, &theme, &candidates, Value::Object(root), &rctx).await {
        if !html.trim().is_empty() {
            // the theme's error page carries the same member-dependent nav
            return (status, [(header::VARY, header::COOKIE.as_str())], Html(html)).into_response();
        }
    }

    // builtin fallback
    (
        status,
        Html(format!(
            "<!DOCTYPE html><html><head><title>{code}</title></head><body style=\"font-family:sans-serif;text-align:center;padding:4rem\"><h1>{code}</h1><p>{}</p></body></html>",
            status.canonical_reason().unwrap_or("Error")
        )),
    )
        .into_response()
}
