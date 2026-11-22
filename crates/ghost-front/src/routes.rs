//! Site request handling: match route → fetch data → render template.
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Redirect, Response};
use ghost_api::AppState;
use ghost_core::models;
use ghost_core::GhostError;
use ghost_db::repos;
use ghost_routing::tree::{RouteMatch, TaxonomyKind};
use serde_json::{json, Value};

use crate::render::{render_template, RenderCtx};
use crate::resolver_now;
use crate::rss;

/// This site's languages. Two settings reads per request, which is what the
/// settings cache is for.
pub(crate) fn site_locales(state: &AppState) -> ghost_core::locales::Locales {
    ghost_core::locales::Locales::from_settings(
        state.settings.get_str("locale").as_deref(),
        state.settings.get("locales_enabled").as_ref(),
    )
}

/// What a page is, for working out which languages it exists in.
#[derive(Clone, Copy)]
pub(crate) enum PageScope<'a> {
    /// A collection, a taxonomy archive or the home page: "the same page in
    /// German" is the same path under the German prefix, because an archive
    /// exists in every language by definition.
    Archive,
    /// A post or page, with its translation group if it has one.
    Entry(Option<&'a str>),
}

/// This page in every language it exists in, for the switcher a theme draws
/// and the `hreflang` alternates `{{ghost_head}}` writes.
///
/// `current` is the locale being rendered.
///
/// Only languages that really have the page are listed. A switcher that
/// offered every enabled language would link to a 404 for the ones that have
/// not been translated yet, which is worse than not offering them: the
/// visitor follows it and concludes the site is broken. So an entry lists
/// only its *published* translations, and an entry with no translation group
/// lists only itself — it is not an archive, and `/de/<its slug>/` is a 404
/// under `render_entry`'s language rule.
pub(crate) async fn translations_for(
    state: &AppState,
    locales: &ghost_core::locales::Locales,
    current: &str,
    scope: PageScope<'_>,
    neutral_path: &str,
) -> Value {
    if !locales.multilingual() {
        return Value::Array(vec![]);
    }
    let mut out = Vec::new();
    for code in locales.enabled() {
        let url = match scope {
            PageScope::Entry(Some(group)) => {
                // Only if a published translation in this language exists,
                // and at that translation's own slug.
                let found = repos::locales::in_locale(&state.db, group, code, locales.default_locale())
                    .await
                    .ok()
                    .flatten()
                    .filter(|post| post.status == "published");
                match found {
                    // The resolver already gives the translation its
                    // language prefix and the base path.
                    Some(post) => {
                        let resolver = resolver_now(state).await;
                        Some(resolver(&models::post_to_json(&post, &[], &[], "")))
                    }
                    None => None,
                }
            }
            // An entry nobody has translated exists in its own language only.
            PageScope::Entry(None) => code
                .eq_ignore_ascii_case(current)
                .then(|| ghost_core::url::base(&locales.localise(neutral_path, code))),
            PageScope::Archive => Some(ghost_core::url::base(&locales.localise(neutral_path, code))),
        };
        if let Some(url) = url {
            out.push(serde_json::json!({
                "locale": code,
                "url": url,
                "active": code.eq_ignore_ascii_case(current),
            }));
        }
    }
    Value::Array(out)
}

fn sitemap_resource_name(path: &str) -> Option<String> {
    let name = path.strip_prefix("/sitemap-")?.strip_suffix(".xml")?;
    if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        Some(name.to_string())
    } else {
        None
    }
}

/// A redirects.yaml answer, carrying the file's own status code: Ghost sends
/// the literal 301/302 the rule was written under
/// (dynamic-redirect-manager.js: `res.redirect(permanent ? 301 : 302)`), not
/// axum's `Redirect::permanent`/`temporary` 308/307 — the number is the point
/// of the rule, not an implementation detail.
fn redirect_response(target: String, permanent: bool) -> Response {
    let status = if permanent {
        StatusCode::MOVED_PERMANENTLY
    } else {
        StatusCode::FOUND
    };
    (status, [(header::LOCATION, target)]).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The theme asset and content-file routes hand this a path segment
    /// straight from the URL, and axum has already percent-decoded it — so
    /// `%2e%2e%2f` reaches here as a real `..` that never looked like
    /// traversal to anything upstream. It served the theme's package.json
    /// until the containment check stopped being `starts_with`, which
    /// compares unresolved components and so could not see the `..` at all.
    #[test]
    fn a_relative_segment_cannot_climb_out_of_the_directory_being_served() {
        let base = std::env::temp_dir().join("ghost-serve-file-test/assets");
        let inside = base.join("built");
        std::fs::create_dir_all(&inside).unwrap();
        std::fs::write(inside.join("screen.css"), "body{}").unwrap();
        // the file one level up that the traversal was reaching
        std::fs::write(base.parent().unwrap().join("package.json"), "{}").unwrap();

        assert!(resolve_within(&base, "built/screen.css").is_some(), "an ordinary asset still resolves");
        for climb in [
            "built/../../package.json",
            "../package.json",
            "built/../../../../../../etc/passwd",
            "/etc/passwd",
            "",
        ] {
            assert!(resolve_within(&base, climb).is_none(), "{climb} must not resolve");
        }

        let _ = std::fs::remove_dir_all(std::env::temp_dir().join("ghost-serve-file-test"));
    }

    #[test]
    fn redirects_answer_with_the_status_code_their_rule_was_written_under() {
        assert_eq!(
            redirect_response("/new/".into(), true).status(),
            StatusCode::MOVED_PERMANENTLY
        );
        assert_eq!(
            redirect_response("/new/".into(), false).status(),
            StatusCode::FOUND
        );
    }
}

/// Main entrypoint for every non-asset site path.
pub async fn entrypoint(State(state): State<AppState>, uri: Uri, headers: HeaderMap) -> Response {
    // The IndexNow key file, which a search engine fetches to confirm that
    // whoever submitted a URL controls this site. Checked here rather than
    // routed: `/{key}.txt` is not a shape this router can express, and a
    // bare `/{key}` would shadow every one-segment permalink. Comparing
    // before answering also means the endpoint cannot be used to read the
    // key — you have to know it to be told it.
    if let Some(requested) = uri.path().strip_prefix('/').and_then(|p| p.strip_suffix(".txt")) {
        if let Some(configured) = ghost_api::announce::indexnow_key(&state) {
            if configured == requested {
                return (
                    [
                        (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
                        (header::CACHE_CONTROL, "public, max-age=3600"),
                    ],
                    configured,
                )
                    .into_response();
            }
        }
    }

    let path = uri.path().to_string();
    let query = uri.query().map(str::to_string);

    // 0. .md content negotiation: /my-post.md → markdown source.
    //
    // Part of what Settings → Meta data's LLM switch controls, so it is off
    // with that switch — otherwise turning the setting off would still leave
    // every post readable as plain Markdown.
    if let Some(md_path) = path.strip_suffix(".md").filter(|_| crate::llms::enabled(&state)) {
        let md_path = format!("{md_path}/");
        // `/bn/my-post.md` is the Bengali post's Markdown: the language
        // prefix comes off before the router sees the path, exactly as for
        // the page itself, and the same rule applies — the entry has to be
        // in the language its URL names.
        let locales = site_locales(&state);
        let (md_locale, md_routed) = locales.split_path(&md_path);
        let router = state.router.read().await.clone();
        if let Some(ghost_routing::tree::RouteMatch::CollectionEntry { slug, .. } | ghost_routing::tree::RouteMatch::StaticPage { slug }) =
            router.match_path(&md_routed)
        {
            let found = match repos::posts::find_by_slug(&state.db, &slug, "post", true).await {
                Ok(Some(p)) => Ok(Some(p)),
                _ => repos::posts::find_by_slug(&state.db, &slug, "page", true).await,
            };
            let found = found.map(|post| {
                post.filter(|p| {
                    !locales.multilingual()
                        || locales.resolve(p.locale.as_deref()).eq_ignore_ascii_case(&md_locale)
                })
            });
            if let Ok(Some(post)) = found {
                if let Some(response)=ghost_api::machine_payments::serve(&state,&post,&uri,&headers).await{return response}
                // Markdown is a public representation, like Ghost's LLM feed:
                // browser membership must never put private bodies into it.
                let html = ghost_api::content::public_html(&post);
                let gated = post.visibility != "public";
                let status = if gated && html.trim().is_empty() { StatusCode::FORBIDDEN } else { StatusCode::OK };
                let mut source = if !gated && html == post.html.as_deref().unwrap_or("") {
                    post.markdown.clone().filter(|m| !m.is_empty()).unwrap_or_else(|| ghost_render::markdown::html_to_plaintext(&html))
                } else { ghost_render::markdown::html_to_plaintext(&html) };
                if gated { source.push_str("\n\nThis content requires a subscription.\n"); }
                return (status, [(header::CONTENT_TYPE, "text/markdown; charset=UTF-8"),
                    (header::CACHE_CONTROL, "private, no-store"), (header::VARY, "Cookie")], source).into_response();
            }
        }
    }

    // /feed/ → /rss/ (Ghost's permanent redirect)
    if path == "/feed" || path == "/feed/" {
        // Ghost's own redirect is a literal 301 (rss-router.js), not the 308
        // axum's `Redirect::permanent` would send.
        return (
            StatusCode::MOVED_PERMANENTLY,
            [(header::LOCATION, ghost_core::url::base("/rss/"))],
        )
            .into_response();
    }

    // 1. redirects.yaml
    {
        let redirects = state.redirects.read().await.clone();
        if let Some((target, permanent)) = redirects.match_path(&path, query.as_deref()) {
            // targets are written in site terms, so an internal one needs the
            // base path; an absolute URL to another host is left alone
            return redirect_response(ghost_core::url::base(&target), permanent);
        }
    }

    // 2. /ghost → admin (mounted separately in the main app). Only paths the
    // admin router did not claim reach this, so it is a 404 rather than a
    // redirect: bouncing to `/ghost/` from here would loop on itself.
    if path == "/ghost" || path == "/ghost/" || path.starts_with("/ghost/") {
        return crate::render::error_page(&state, StatusCode::NOT_FOUND, &path, headers).await;
    }

    // 3. per-resource sitemaps (/sitemap-posts.xml …). The headers go with it:
    // they decide whether this is a person opening the URL, who gets a page,
    // or a crawler, who gets the XML.
    if let Some(resource) = sitemap_resource_name(&path) {
        return crate::sitemap::sitemap_resource(state, headers, resource).await;
    }

    // 4. the language prefix, before the router sees the path.
    //
    // `/de/my-post/` is routed as `/my-post/` and rendered in German. The
    // default language is unprefixed, so on a single-language site — and for
    // every URL an existing site already has — this is a no-op and the
    // original path goes to the router untouched. See
    // `ghost_core::locales::split_path` for why an unknown prefix is treated
    // as an ordinary path segment rather than a language.
    //
    // `path` stays the prefixed form: it is what the canonical link, the
    // `og:url` and the pagination links are built from, and those have to
    // name the URL the visitor is actually on.
    let locales = site_locales(&state);
    let (locale, routed) = locales.split_path(&path);

    // 5. router tree
    let router = state.router.read().await.clone();
    let Some(route) = router.match_path(&routed) else {
        return crate::render::error_page(&state, StatusCode::NOT_FOUND, &path, headers).await;
    };

    match route {
        RouteMatch::Preview { uuid, .. } => render_preview(state, uuid, &path, query.as_deref(), headers).await,
        RouteMatch::Rss { kind, name, .. } => rss::render_rss(state, kind, name, &path, &locale, &headers).await,
        RouteMatch::CollectionIndex { name, filter, order, limit, templates, data, page, .. } => {
            render_collection(state, name, filter, order, limit, templates, data, page, &path, &locale, headers).await
        }
        RouteMatch::Channel { name, filter, order, limit, templates, page, .. } => {
            render_collection(state, name, filter, order, limit, templates, None, page, &path, &locale, headers).await
        }
        RouteMatch::CollectionEntry { slug, .. } => render_entry(state, slug, &path, &locale, query.as_deref(), headers).await,
        RouteMatch::StaticPage { slug } => render_entry(state, slug, &path, &locale, query.as_deref(), headers).await,
        RouteMatch::Taxonomy { kind, slug, page, .. } => {
            render_taxonomy(state, kind, slug, page, &path, &locale, headers).await
        }
        RouteMatch::Custom { name, templates, data, content_type } => {
            render_custom(state, name, templates, data, content_type, &path, headers).await
        }
    }
}

/// Serialize a page of posts with relations resolved (one query per page).
///
/// `member` is the `@member` context, which decides each post's `access`
/// flag — a listing shows a lock on a post the visitor cannot read, and the
/// answer depends on who is asking.
pub(crate) async fn posts_page_json(
    state: &AppState,
    posts: &[ghost_core::models::Post],
    member: &Value,
) -> Vec<Value> {
    let rel = repos::posts::load_relations(&state.db, posts).await.unwrap_or_default();
    let resolver = resolver_now(state).await;
    let status = crate::render::member_status(member);
    let tiers = crate::render::member_tiers(member);
    posts
        .iter()
        .map(|p| {
            let authors = rel.authors_for(&p.id);
            let tags = rel.tags_for(&p.id);
            let mut j = models::post_to_json(p, &authors, &tags, "");
            j["url"] = json!(resolver(&j));
            ghost_render::util::resolve_nested_urls(&mut j, &*resolver);
            crate::render::apply_member_tier_access(&mut j, status, &tiers);
            j
        })
        .collect()
}

/// Serialize a single post with relations.
/// `@page`: the page itself plus the flag themes read as
/// `@page.show_title_and_feature_image` to decide whether to draw the
/// title and feature image. Ghost puts the flag only here and not in the
/// Content API's page JSON, so it is added here and not in `post_to_json`.
fn page_data(post_json: &Value, show_title_and_feature_image: bool) -> Value {
    let mut page = post_json.clone();
    if let Value::Object(m) = &mut page {
        m.insert("show_title_and_feature_image".into(), Value::Bool(show_title_and_feature_image));
    }
    page
}

async fn post_json(
    state: &AppState,
    post: &ghost_core::models::Post,
    url: Option<&str>,
    member: &Value,
) -> Value {
    let rel: ghost_db::repos::PostRelations =
        repos::posts::load_relations(&state.db, std::slice::from_ref(post)).await.unwrap_or_default();
    let authors = rel.authors_for(&post.id);
    let tags = rel.tags_for(&post.id);
    // Tools → Page Builder: a post with a layout switched on — or one a
    // single-post template applies to — is drawn from the builder. Swapped
    // in before the access rules below, so a paywall still cuts a builder
    // page exactly as it cuts its own content.
    let built;
    let entry = crate::builder_site::entry_html(state, post).await;
    let original = post.html.clone();
    let post = match &entry {
        Some((html, _)) => {
            let mut p = post.clone();
            p.html = Some(format!("{}{html}", crate::builder_site::asset_tags()));
            built = p;
            &built
        }
        None => post,
    };
    let mut j = models::post_to_json(post, &authors, &tags, url.unwrap_or(""));
    let resolver = resolver_now(state).await;
    if url.is_none() {
        j["url"] = json!(resolver(&j));
    }
    ghost_render::util::resolve_nested_urls(&mut j, &*resolver);
    if let Some((_, from_template)) = &entry {
        let path = j["url"].as_str().and_then(|u| u.split_once("://").map(|(_, r)| r)).and_then(|r| r.find('/').map(|i| r[i..].to_string())).unwrap_or_else(|| "/".into());
        let html = j["html"].as_str().unwrap_or("").to_string();
        let orig = if *from_template { original.as_deref() } else { None };
        let expanded = crate::builder_site::expand(state, &html, Some(&j), orig, member, &path).await;
        j["html"] = json!(expanded);
    }
    crate::render::apply_member_tier_access(&mut j, crate::render::member_status(member), &crate::render::member_tiers(member));
    j
}

async fn render_collection(
    state: AppState,
    name: String,
    filter: Option<ghost_nql::Expr>,
    order: Option<String>,
    limit: Option<i64>,
    templates: Vec<String>,
    data: Option<serde_yaml::Value>,
    page: i64,
    path: &str,
    locale: &str,
    headers: HeaderMap,
) -> Response {
    let theme = state.theme.read().await.clone();
    let limit_final = limit.unwrap_or(theme.posts_per_page());
    let locales = site_locales(&state);

    let q = repos::posts::BrowsePosts {
        filter,
        order,
        limit: limit_final,
        page,
        post_type: Some("post".into()),
        public: true,
        include_authors: true,
        include_tags: true,
        // An archive shows one language's posts. Suppressed entirely on a
        // single-language site, so the query a existing install runs is
        // byte for byte the one it ran before.
        locale: locales.multilingual().then(|| locale.to_string()),
        default_locale: Some(locales.default_locale().to_string()),
    };
    let result = match repos::posts::browse(&state.db, &q).await {
        Ok(r) => r,
        Err(e) => return error_response(e),
    };

    if page > 1 {
        let pages = (result.total + result.limit.max(1) - 1) / result.limit.max(1);
        if page > pages.max(1) {
            return crate::render::error_page(&state, StatusCode::NOT_FOUND, path, headers).await;
        }
    }

    let member = crate::render::member_context(&state, &headers).await;
    let posts_json = posts_page_json(&state, &result.posts, &member).await;

    // data: entries (e.g. data: page.blog)
    let mut extra = serde_json::Map::new();
    if let Some(yaml) = &data {
        if let Some(s) = yaml.as_str() {
            if let Some(data_json) = resolve_data_entry(&state, s, &member).await {
                extra.insert(data_entry_key(s), data_json);
            }
        }
    }

    let pagination = models::pagination_json(result.page, result.limit.max(1), result.total);
    let mut context: Vec<String> = Vec::new();
    if page > 1 {
        context.push("paged".into());
    }
    // Each language's home page is a home page: `/bn/` gets the `home`
    // context and the theme's `home.hbs` exactly as `/` does, so every
    // language's front page has the same design with no theme change. The
    // URL prefix is stripped before asking; the canonical and the schema
    // still use the real path, so `/bn/` remains its own page to a crawler.
    let routed = locales.split_path(path).1;
    if routed == "/" {
        context.push("home".into());
    }
    context.push(name.clone());

    let mut root = serde_json::Map::new();
    root.insert("posts".into(), Value::Array(posts_json));
    root.insert("pagination".into(), pagination);
    root.extend(extra);

    let is_front = routed == "/" && page == 1;
    let candidates = {
        let router = state.router.read().await.clone();
        router.collection_templates(&name, &templates, is_front)
    };

    // An archive exists in every language by definition, so the switcher
    // offers them all at the same path under each prefix. `paginated_base`
    // is not used here: the path is already language-neutral, because the
    // prefix was stripped before the router ran.
    let neutral = locales.split_path(path).1;
    let translations = translations_for(&state, &locales, locale, PageScope::Archive, &neutral).await;
    let site_overrides = if routed == "/" { home_details(&state, &locales, locale).await } else { Default::default() };
    let rctx = RenderCtx {
        context,
        relative_url: path.to_string(),
        member,
        locale: locale.to_string(),
        translations,
        site_overrides,
    };
    match render_template(&state, &theme, &candidates, Value::Object(root), &rctx).await {
        Ok(html) => html_response(html),
        Err(e) => error_response(e),
    }
}

/// A language's home-page details, from the published page whose slug is
/// that language's code — the page slugged `bn` for `/bn/`.
///
/// That page could never be opened as a page anyway: `/bn/` is the Bengali
/// home, read before any slug. So it does a job instead. Its fields replace
/// the site-wide ones on that home page only, which is how a language gets
/// its own `<title>`, description and share cards without a setting per
/// language, and in the editor everybody already knows.
///
/// Only fields that are filled in replace anything; the rest keep the
/// site's. Looked up by code for every language, the default included, so a
/// change of default language needs nothing re-done. Empty on a
/// single-language site.
pub(crate) async fn home_details(
    state: &AppState,
    locales: &ghost_core::locales::Locales,
    locale: &str,
) -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    if !locales.multilingual() {
        return out;
    }
    let Ok(Some(page)) = repos::posts::find_by_slug(&state.db, &locale.to_ascii_lowercase(), "page", true).await else {
        return out;
    };
    let j = models::post_to_json(&page, &[], &[], "");
    let field = |key: &str| j.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let first = |keys: &[&str]| keys.iter().find_map(|k| field(k));
    let mut put = |key: &str, value: Option<String>| {
        if let Some(v) = value {
            out.insert(key.to_string(), Value::String(v));
        }
    };
    // `<title>` and the meta description.
    put("meta_title", first(&["meta_title", "title"]));
    put("meta_description", first(&["meta_description", "custom_excerpt"]));
    // What a theme prints under the site name in its header.
    put("description", first(&["custom_excerpt", "meta_description"]));
    // The share cards: on a home page these are read from the site, and
    // fall back to the site's title — which is the default language's.
    put("og_title", first(&["og_title", "meta_title", "title"]));
    put("og_description", first(&["og_description", "meta_description", "custom_excerpt"]));
    put("og_image", first(&["og_image", "feature_image"]));
    put("twitter_title", first(&["twitter_title", "meta_title", "title"]));
    put("twitter_description", first(&["twitter_description", "meta_description", "custom_excerpt"]));
    put("twitter_image", first(&["twitter_image", "feature_image"]));
    put("cover_image", field("feature_image"));
    out
}

fn data_entry_key(short: &str) -> String {
    short.split('.').next().unwrap_or("data").to_string()
}

/// Resolve a `data:` route entry (`data: page.blog`).
///
/// `member` is threaded through so a `data:`-loaded post is gated for the
/// same visitor as the page embedding it.
async fn resolve_data_entry(state: &AppState, short: &str, member: &Value) -> Option<Value> {
    let (resource, slug) = short.split_once('.')?;
    let resolver = resolver_now(state).await;
    match resource {
        "page" | "post" => {
            let post_type = if resource == "page" { "page" } else { "post" };
            let p = repos::posts::find_by_slug(&state.db, slug, post_type, true).await.ok()??;
            Some(post_json(state, &p, None, member).await)
        }
        "tag" => {
            let t = repos::tags::find_by_slug(&state.db, slug).await.ok()??;
            let mut j = models::tag_to_json(&t, None);
            let url = resolver(&j);
            if let Value::Object(m) = &mut j {
                m.insert("url".into(), json!(url));
            }
            Some(j)
        }
        "author" => {
            let u = repos::users::find_by_slug(&state.db, slug).await.ok()??;
            let mut j = models::user_public_json(&u, None);
            let url = resolver(&j);
            if let Value::Object(m) = &mut j {
                m.insert("url".into(), json!(url));
            }
            Some(j)
        }
        _ => None,
    }
}

async fn render_entry(state: AppState, slug: String, path: &str, locale: &str, query: Option<&str>, headers: HeaderMap) -> Response {
    // posts first, then pages (Ghost's router fallthrough)
    let (post, post_type) = match repos::posts::find_by_slug(&state.db, &slug, "post", true).await {
        Ok(Some(p)) => (p, "post"),
        _ => match repos::posts::find_by_slug(&state.db, &slug, "page", true).await {
            Ok(Some(p)) => (p, "page"),
            _ => {
                // Last chance before a 404: an offer code. Ghost hands these
                // out as plain `site.com/<code>` links and sends the visitor
                // to the homepage with Portal opened on the offer. A post or
                // page of the same slug wins, which is why this is checked
                // only once both have missed.
                if let Ok(Some(offer)) =
                    repos::offers::find_redeemable_by_code(&state.db, &slug).await
                {
                    return Redirect::to(&ghost_core::url::base(&format!(
                        "/#/portal/offers/{}",
                        offer.id
                    )))
                    .into_response();
                }
                return crate::render::error_page(&state, StatusCode::NOT_FOUND, path, headers).await;
            }
        },
    };

    // The entry has to be in the language the URL asked for.
    //
    // `/de/my-post/` naming an English post is a 404 **in German**, not the
    // English post served under a German URL: two URLs carrying the same
    // text is the duplicate-content signal that `hreflang` exists to avoid,
    // and a German URL returning English reads as broken. The same rule in
    // the other direction keeps a German post off the unprefixed path.
    //
    // A NULL locale is the default language — never backfilled, so this is
    // also what makes every pre-existing post keep answering on its own URL.
    let locales = site_locales(&state);
    if locales.multilingual() {
        let entry_locale = post.locale.as_deref().unwrap_or(locales.default_locale());
        if !entry_locale.eq_ignore_ascii_case(locale) {
            return crate::render::error_page(&state, StatusCode::NOT_FOUND, path, headers).await;
        }
    }

    let member = crate::render::member_context(&state, &headers).await;
    let gift_token = form_urlencoded::parse(query.unwrap_or("").as_bytes()).find(|(key,_)|key=="gift").map(|(_,value)|value.into_owned());
    let gifted = if let Some(token) = &gift_token { ghost_api::publishing::valid_gift(&state,&post.id,token).await } else { false };
    let post_member = if gifted { json!({"status":"paid","tiers":post.tier_ids}) } else { member.clone() };
    let post_json = post_json(&state, &post, None, &post_member).await;
    let mut context = vec![post_type.to_string()];
    if post_type == "page" {
        context.push("page".into());
    }

    let mut root = serde_json::Map::new();
    if post_type == "page" {
        root.insert("page".into(), page_data(&post_json, post.show_title_and_feature_image));
        root.insert("post".into(), post_json);
    } else {
        root.insert("post".into(), post_json);
    }

    let theme = state.theme.read().await.clone();
    let mut candidates = vec![format!("post-{slug}")];
    if let Some(ct) = &post.custom_template {
        candidates.push(ct.clone());
    }
    if post_type == "page" {
        candidates.push(format!("page-{slug}"));
        candidates.push("page".into());
    }
    candidates.push("post".into());
    candidates.push("index".into());

    let rctx = RenderCtx {
        context,
        relative_url: path.to_string(),
        member,
        locale: locale.to_string(),
        // Only the languages this entry really has, read from its
        // translation group — so the switcher cannot offer a link to a
        // language that would 404 under the rule above.
        translations: translations_for(
            &state,
            &locales,
            locale,
            PageScope::Entry(post.translation_group.as_deref()),
            &locales.split_path(path).1,
        )
        .await,
        site_overrides: Default::default(),
    };
    match render_template(&state, &theme, &candidates, Value::Object(root), &rctx).await {
        Ok(html) => {
            let mut response = html_response(html);
            if gift_token.is_some() {
                response.headers_mut().insert(header::CACHE_CONTROL, "private, no-store".parse().unwrap());
                response.headers_mut().insert("referrer-policy", "no-referrer".parse().unwrap());
            }
            response
        },
        Err(e) => error_response(e),
    }
}

async fn render_taxonomy(
    state: AppState,
    kind: TaxonomyKind,
    slug: String,
    page: i64,
    path: &str,
    locale: &str,
    headers: HeaderMap,
) -> Response {
    let theme = state.theme.read().await.clone();
    let locales = site_locales(&state);
    let resolver = resolver_now(&state).await;

    let (taxonomy_json, name, filter, resource_label): (Value, String, Option<ghost_nql::Expr>, &'static str) =
        match kind {
            TaxonomyKind::Tag => {
                let t = match repos::tags::find_by_slug(&state.db, &slug).await {
                    Ok(Some(t)) if t.visibility == "public" => t,
                    _ => {
                        return crate::render::error_page(&state, StatusCode::NOT_FOUND, path, headers).await;
                    }
                };
                // A tag locked to a language has an archive only in that
                // language. A tag with no locale belongs to every language —
                // which is what a brand name or an internal tag is, and what
                // every tag on a site that has not enabled a second
                // language is.
                if locales.multilingual() {
                    if let Some(tag_locale) = t.locale.as_deref().filter(|c| !c.is_empty()) {
                        if !tag_locale.eq_ignore_ascii_case(locale) {
                            return crate::render::error_page(&state, StatusCode::NOT_FOUND, path, headers).await;
                        }
                    }
                }
                let mut j = models::tag_to_json(&t, None);
                // This archive, in the language it is being read in.
                let url = ghost_render::util::resolve_for_page(&j, Some(locale), true, &*resolver);
                if let Value::Object(m) = &mut j {
                    m.insert("url".into(), json!(url));
                }
                (j, "tag".to_string(), ghost_nql::parse(&format!("tags:'{slug}'")).ok(), "tag")
            }
            TaxonomyKind::Author => {
                let u = match repos::users::find_by_slug(&state.db, &slug).await {
                    Ok(Some(u)) if u.status.as_deref() == Some("active") => u,
                    _ => {
                        return crate::render::error_page(&state, StatusCode::NOT_FOUND, path, headers).await;
                    }
                };
                let mut j = models::user_public_json(&u, None);
                let url = ghost_render::util::resolve_for_page(&j, Some(locale), false, &*resolver);
                if let Value::Object(m) = &mut j {
                    m.insert("url".into(), json!(url));
                }
                (j, "author".to_string(), ghost_nql::parse(&format!("authors:'{slug}'")).ok(), "author")
            }
        };

    let q = repos::posts::BrowsePosts {
        filter,
        order: None,
        limit: theme.posts_per_page(),
        page,
        post_type: Some("post".into()),
        public: true,
        include_authors: true,
        include_tags: true,
        // A tag or author archive lists one language's posts, like any other
        // archive — otherwise `/de/author/ada/` would list her English work.
        locale: locales.multilingual().then(|| locale.to_string()),
        default_locale: Some(locales.default_locale().to_string()),
    };
    let result = match repos::posts::browse(&state.db, &q).await {
        Ok(r) => r,
        Err(e) => return error_response(e),
    };

    let member = crate::render::member_context(&state, &headers).await;
    let posts_json = posts_page_json(&state, &result.posts, &member).await;

    let pagination = models::pagination_json(result.page, result.limit.max(1), result.total);
    let mut context: Vec<String> = Vec::new();
    if page > 1 {
        context.push("paged".into());
    }
    context.push(name.clone());

    let mut root = serde_json::Map::new();
    root.insert(resource_label.into(), taxonomy_json);
    root.insert("posts".into(), Value::Array(posts_json));
    root.insert("pagination".into(), pagination);

    let candidates = vec![format!("{name}-{slug}"), name.clone(), "index".into()];
    let neutral = locales.split_path(path).1;
    let translations = translations_for(&state, &locales, locale, PageScope::Archive, &neutral).await;
    let rctx = RenderCtx {
        context,
        locale: locale.to_string(),
        translations,
        relative_url: path.to_string(),
        member,
        ..Default::default()
    };
    match render_template(&state, &theme, &candidates, Value::Object(root), &rctx).await {
        Ok(html) => html_response(html),
        Err(e) => error_response(e),
    }
}

async fn render_custom(
    state: AppState,
    name: String,
    templates: Vec<String>,
    data: Option<serde_yaml::Value>,
    content_type: Option<String>,
    path: &str,
    _headers: HeaderMap,
) -> Response {
    let theme = state.theme.read().await.clone();
    let member = crate::render::member_context(&state, &_headers).await;
    let mut root = serde_json::Map::new();
    if let Some(yaml) = &data {
        if let Some(s) = yaml.as_str() {
            if let Some(d) = resolve_data_entry(&state, s, &member).await {
                root.insert(data_entry_key(s), d);
            }
        }
    }
    let mut candidates = templates.clone();
    candidates.push(name.clone());
    let rctx = RenderCtx {
        context: vec![name],
        relative_url: path.to_string(),
        member,
        ..Default::default()
    };
    match render_template(&state, &theme, &candidates, Value::Object(root), &rctx).await {
        Ok(html) => {
            let mut resp = html_response(html);
            if let Some(ct) = content_type {
                if let Ok(v) = ct.parse() {
                    resp.headers_mut().insert(header::CONTENT_TYPE, v);
                }
            }
            resp
        }
        Err(e) => error_response(e),
    }
}

/// Rendered pages depend on the member session: the same URL yields "Sign in"
/// for a visitor and "Account" for a member. `Vary: Cookie` is what stops a
/// shared cache in front of this server from handing one visitor's variant to
/// another.
fn html_response(mut html: String) -> Response {
    let script=format!("<script src=\"{}\" defer></script>",ghost_core::url::base("/public/staff-toolbar.js"));
    if let Some(index)=html.rfind("</body>"){html.insert_str(index,&script);}else{html.push_str(&script);}
    (
        [(header::VARY, header::COOKIE.as_str())],
        Html(html),
    )
        .into_response()
}

async fn render_preview(state: AppState, uuid: String, path: &str, query: Option<&str>, headers: HeaderMap) -> Response {
    // P1: previews are open (admin gating arrives with the admin API in P3)
    let Some(post) = repos::posts::find_by_uuid(&state.db, &uuid).await.unwrap_or(None) else {
        return crate::render::error_page(&state, StatusCode::NOT_FOUND, path, headers).await;
    };
    // passed in rather than resolved, so it needs the base path putting on
    // here the way the resolver would have
    let preview_url = ghost_core::url::base(&format!("/p/{uuid}/"));
    let member = crate::render::member_context(&state, &headers).await;
    let preview_member=json!({"status":"paid","tiers":post.tier_ids});
    let mut post_json = post_json(&state, &post, Some(&preview_url), &preview_member).await;
    // The page builder's canvas: the page as the theme draws it, with an
    // empty layout root where the content goes. The editor (same origin)
    // draws into it. Holds nothing the preview does not already show.
    let canvas = form_urlencoded::parse(query.unwrap_or("").as_bytes()).any(|(k, v)| k == "vb" && v == "canvas");
    if canvas {
        let settings = ghost_core::builder::Settings::read(|k| state.settings.get_str(k));
        post_json["html"] = json!(format!(
            r#"{}<div class="vb-page vb-editing" data-vb-canvas style="{}"></div>"#,
            crate::builder_site::asset_tags(),
            ghost_core::builder::esc(&settings.css_vars()),
        ));
    }

    match render_preview_snapshot(&state, &post, post_json, member, path).await {
        Ok(html) if canvas => html_response(strip_overlays(&html)),
        Ok(html) => html_response(html),
        Err(e) => error_response(e),
    }
}

/// The page builder's canvas without what floats over a page: popups, the
/// cookie banner, ads, Portal and the staff toolbar. They would cover the
/// layout being edited; the published page keeps every one of them.
fn strip_overlays(html: &str) -> String {
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r#"(?is)<script\b[^>]*\bsrc="[^"]*(?:/popups/api/|/cookies/api/|/ads/api/|staff-toolbar\.js|portal\.min\.js)[^"]*"[^>]*>\s*</script>"#).unwrap()
    });
    RE.replace_all(html, "").into_owned()
}

#[cfg(test)]
mod canvas_tests {
    #[test]
    fn the_canvas_drops_overlay_scripts_only() {
        let html = r#"<script src="/popups/api/assets/1/popups.js"></script><script src="/cookies/api/assets/2/cookies.js"></script><script defer src="https://cdn.jsdelivr.net/ghost/portal@~2.72/umd/portal.min.js" data-x="1"></script><script src="/assets/built/theme.js"></script>"#;
        let out = super::strip_overlays(html);
        assert_eq!(out, r#"<script src="/assets/built/theme.js"></script>"#);
    }
}

/// Use the active theme for a transient post, without persisting editor changes.
pub(crate) async fn render_preview_snapshot(
    state: &AppState,
    post: &ghost_core::models::Post,
    mut post_json: Value,
    member: Value,
    path: &str,
) -> ghost_core::Result<String> {
    post_json["access"] = json!(true);

    let mut context = vec!["preview".to_string()];
    let label = if post.post_type == "page" { "page" } else { "post" };
    context.push(label.to_string());

    let mut root = serde_json::Map::new();
    if post.post_type == "page" {
        root.insert("page".into(), page_data(&post_json, post.show_title_and_feature_image));
        root.insert("post".into(), post_json);
    } else {
        root.insert("post".into(), post_json);
    }

    let theme = state.theme.read().await.clone();
    let mut candidates = if post.post_type == "page" {
        vec!["page".to_string(), "post".into()]
    } else {
        vec!["post".to_string()]
    };
    if let Some(custom) = post.custom_template.as_ref().filter(|name|!name.is_empty()) {
        candidates.insert(0,custom.clone());
    }
    let rctx = RenderCtx {
        context,
        relative_url: path.to_string(),
        member,
        ..Default::default()
    };
    render_template(state, &theme, &candidates, Value::Object(root), &rctx).await
}

/// Render `html` as a page of the site, in the active theme — for screens
/// that are not posts or pages in the database (the public directory). A
/// transient page object goes through the theme's `page` template exactly as
/// a preview does, so every theme draws it with its own header, footer and
/// typography without knowing it exists.
/// [`render_virtual_page`] without the theme's title and feature image —
/// for a builder page standing in for the whole site (maintenance mode).
pub(crate) async fn render_virtual_page_hidden(state: &AppState, headers: &HeaderMap, path: &str, title: &str, html: String) -> Response {
    render_virtual_page_with(state, headers, path, title, "", html, None, false).await
}

pub(crate) async fn render_virtual_page(
    state: &AppState,
    headers: &HeaderMap,
    path: &str,
    title: &str,
    excerpt: &str,
    html: String,
    image: Option<String>,
) -> Response {
    render_virtual_page_with(state, headers, path, title, excerpt, html, image, true).await
}

#[allow(clippy::too_many_arguments)]
async fn render_virtual_page_with(
    state: &AppState,
    headers: &HeaderMap,
    path: &str,
    title: &str,
    excerpt: &str,
    html: String,
    image: Option<String>,
    show_title: bool,
) -> Response {
    let member = crate::render::member_context(state, headers).await;
    let post = ghost_core::models::Post {
        id: format!("virtual{}", ghost_core::ids::new_id()),
        uuid: Some(ghost_core::ids::uuid_v4()),
        title: Some(title.to_string()),
        slug: Some(path.trim_matches('/').replace('/', "-")),
        html: Some(html),
        post_type: "page".into(),
        status: "published".into(),
        visibility: "public".into(),
        custom_excerpt: (!excerpt.is_empty()).then(|| excerpt.to_string()),
        feature_image: image,
        // Themes draw a page's title and feature image only when this is
        // on; a directory page wants both.
        show_title_and_feature_image: show_title,
        published_at: Some(ghost_core::dates::now_db()),
        created_at: Some(ghost_core::dates::now_db()),
        updated_at: Some(ghost_core::dates::now_db()),
        ..Default::default()
    };
    let url = ghost_core::url::base(path);
    let mut json = post_json(state, &post, Some(&url), &member).await;
    json["access"] = json!(true);
    let mut root = serde_json::Map::new();
    root.insert("page".into(), page_data(&json, show_title));
    root.insert("post".into(), json);
    let theme = state.theme.read().await.clone();
    let rctx = RenderCtx { context: vec!["page".into()], relative_url: path.to_string(), member, ..Default::default() };
    match render_template(state, &theme, &["page".to_string(), "post".into()], Value::Object(root), &rctx).await {
        Ok(html) => html_response(html),
        Err(e) => error_response(e),
    }
}

fn error_response(e: GhostError) -> Response {
    tracing::error!("render error: {e}");
    (
        StatusCode::from_u16(e.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        Html(format!("<h1>Something went wrong</h1><p>{}</p>", e.message())),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// static assets
// ---------------------------------------------------------------------------

/// Resolve `rel` inside `base`, or `None` if it points anywhere else.
///
/// `base.join(rel).starts_with(base)` looks like the containment check and is
/// not one: `starts_with` compares path *components* without resolving them,
/// so `assets/../../package.json` still begins with the `assets` component
/// and passes, while the filesystem happily walks the `..` back out. The
/// router does not save it either — axum percent-decodes a wildcard capture,
/// so `%2e%2e%2f` arrives as `../` having never looked like traversal to
/// anything in front of it.
///
/// So the segments are walked instead: `..` is refused outright rather than
/// popped, since nothing under a theme or the content directory has a
/// legitimate reason to name a parent, and the resolved path is then checked
/// against the canonical base to catch a symlink pointing out of the tree.
fn resolve_within(base: &std::path::Path, rel: &str) -> Option<std::path::PathBuf> {
    use std::path::Component;
    let mut safe = std::path::PathBuf::new();
    for component in std::path::Path::new(rel).components() {
        match component {
            Component::Normal(part) => safe.push(part),
            Component::CurDir => continue,
            // ParentDir, RootDir and Prefix are all refusals.
            _ => return None,
        }
    }
    if safe.as_os_str().is_empty() {
        return None;
    }
    let full = base.join(&safe);
    // Belt and braces: a symlink inside the tree could still point out of it.
    match (full.canonicalize(), base.canonicalize()) {
        (Ok(f), Ok(b)) if f.starts_with(&b) => Some(full),
        _ => None,
    }
}

fn serve_file(base: &std::path::Path, rel: &str, cache: Option<&str>) -> Response {
    let Some(file_path) = resolve_within(base, rel) else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    if !file_path.is_file() {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    match std::fs::read(&file_path) {
        Ok(bytes) => {
            let mime = mime_guess::from_path(&file_path).first_or_octet_stream();
            let mut builder = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime.as_ref());
            // An SVG is a document, not a picture: it may carry <script>, and
            // uploading one only takes a Contributor. Opened directly it would
            // run that script on this site's origin, which is enough to drive
            // the admin API as whoever followed the link. `sandbox` puts the
            // response in an opaque origin, so the script has nothing to act
            // on. Nothing legitimate is lost — an SVG referenced as an <img>
            // never ran script in the first place.
            if mime.as_ref() == "image/svg+xml" {
                builder = builder
                    .header("content-security-policy", "sandbox")
                    .header("x-content-type-options", "nosniff");
            }
            if let Some(cache) = cache {
                builder = builder.header(header::CACHE_CONTROL, cache);
            }
            builder.body(axum::body::Body::from(bytes)).unwrap()
        }
        Err(_) => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

pub async fn theme_asset(State(state): State<AppState>, Path(path): Path<String>) -> Response {
    let theme = state.theme.read().await.clone();
    serve_file(&theme.dir.join("assets"), &path, Some("public, max-age=31536000, immutable"))
}

/// /content/images/... — accepts Ghost's `/size/WxH/` notation (serves the
/// original image; resizing is a later-phase enhancement).
pub(crate) fn original_image(state: &AppState, path: &str) -> Response {
    serve_file(&state.config.images_dir(), path, None)
}

pub async fn content_media(State(state): State<AppState>, Path(path): Path<String>) -> Response {
    serve_file(&state.config.media_dir(), &path, None)
}

pub async fn content_file(State(state): State<AppState>, Path(path): Path<String>) -> Response {
    serve_file(&state.config.files_dir(), &path, None)
}


pub async fn public_file(State(state): State<AppState>, Path(path): Path<String>) -> Response {
    serve_file(&state.config.content_path().join("public"), &path, None)
}

pub async fn favicon(State(state): State<AppState>) -> Response {
    let icon = state.settings.get_str("icon").unwrap_or_default();
    if !icon.is_empty() {
        if icon.starts_with("http") {
            // Ghost redirects the favicon route with 302 (serve-favicon.js).
            return redirect_response(icon, false);
        }
        // `icon` is stored as a site path (`/content/images/x.png`), and
        // `content_path()` is already the `content/` directory — so the
        // leading `content/` segment has to come off or the lookup lands on
        // `content/content/images/…`. A value that arrived with the base path
        // on it is reduced to the stored form first.
        let cleaned = ghost_core::url::strip(&icon).trim_start_matches('/').to_string();
        let cleaned = cleaned.strip_prefix("content/").unwrap_or(&cleaned);
        let base = state.config.content_path();
        let file_path = base.join(cleaned);
        if file_path.is_file() {
            if let Ok(bytes) = std::fs::read(&file_path) {
                let mime = mime_guess::from_path(&file_path).first_or_octet_stream();
                return Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, mime.as_ref())
                    .body(axum::body::Body::from(bytes))
                    .unwrap();
            }
        }
    }
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><circle cx="16" cy="16" r="16" fill="#15171A"/><text x="16" y="22" font-family="Arial" font-size="18" fill="white" text-anchor="middle">G</text></svg>"##;
    ([(header::CONTENT_TYPE, "image/svg+xml")], svg).into_response()
}
