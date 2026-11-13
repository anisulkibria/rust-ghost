//! Settings -> Advanced: code injection, migration, exports, imports, redirects, history and the danger zone.

use super::*;

// --- advanced: integrations (existing page), import/export, code injection, labs ---

pub(crate) async fn advanced_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };

    // Beta and private feature flags used to sit here; they live on the
    // Features page now, where the rest of what a publisher can switch on and
    // off already is. What is left on this page is plumbing — import, export,
    // redirects, routes — and none of it posts a form, so the header carries
    // no Save.
    let group = SETTINGS_GROUPS.iter().find(|g| g.slug == "advanced").unwrap();
    page_for_role(actor_role,
        "Advanced",
        "settings",
        Some(&user),
        &format!(
            r#"{head}{banner}
<div class="section">Redirects &amp; routes</div>
<div class="card rows rise" style="--i:2">
<div class="row" style="align-items:flex-start"><span class="ic tint-red">{redirects_ic}</span>
<div class="body" style="width:100%"><span class="title" style="display:block;margin-bottom:8px">Redirects</span>
<div style="display:flex;gap:12px;align-items:center;flex-wrap:wrap">
<a class="btn tonal small" href="/ghost/settings/advanced/redirects/">Download</a>
<form method="post" action="/ghost/settings/advanced/redirects/" enctype="multipart/form-data" style="display:flex;gap:12px;align-items:center;flex-wrap:wrap;margin:0">
<input type="file" name="file" accept=".yaml,.yml,.json" required class="filefield">
<button class="btn tonal small" type="submit">Upload redirects</button></form></div>
<p class="muted" style="margin:10px 0 0">Ghost-format redirects, YAML or JSON — <code>301: /old/ -> /new/</code>. The file is validated before anything takes effect.</p></div></div>
<div class="row" style="align-items:flex-start"><span class="ic tint-red">{routes_ic}</span>
<div class="body" style="width:100%"><span class="title" style="display:block;margin-bottom:8px">Routes</span>
<div style="display:flex;gap:12px;align-items:center;flex-wrap:wrap">
<a class="btn tonal small" href="/ghost/settings/advanced/routes/">Download</a>
<form method="post" action="/ghost/settings/advanced/routes/" enctype="multipart/form-data" style="display:flex;gap:12px;align-items:center;flex-wrap:wrap;margin:0">
<input type="file" name="file" accept=".yaml,.yml" required class="filefield">
<button class="btn tonal small" type="submit">Upload routes</button></form></div>
<p class="muted" style="margin:10px 0 0">routes.yaml defines the site's URL structure — collections, channels and custom templates. Uploading rebuilds routing and the active theme immediately.</p></div></div>
</div>
<p class="muted" style="margin:14px 2px 0">Looking for something else? Beta and private features are in <a href="/ghost/settings/features/">Settings → Features</a>, and importing and exporting moved to <a href="/ghost/settings/migration/">Settings → Migration tools</a>.</p>"#,
            head = group_head(group, None),
            banner = flash_banner(&q),
            redirects_ic = svg("link"),
            routes_ic = svg("layout"),
        ),
        true,
    )
    .into_response()
}

// --- migration tools: import and export ------------------------------------

/// The platforms a publisher most often arrives from, and what to feed the
/// universal importer with in each case.
///
/// Ghost answers this with a hosted service at `migrate.ghost.org`, embedded
/// in an iframe and handed an Admin API key. A self-hosted port cannot stand
/// that service up, and handing a third party an owner-level key to do it
/// would be a poor trade — so these are directions to the file each platform
/// exports, which the universal importer below reads.
pub(crate) const MIGRATION_SOURCES: &[(&str, &str)] = &[
    ("Ghost", "Settings → Migration tools → Export on the old site gives the JSON or zip this screen reads directly."),
    ("WordPress", "Install the Ghost plugin on WordPress and use its export, which writes a Ghost-format zip."),
    ("Substack", "Settings → Exports → New export. Unzip it; the posts CSV imports here, the rest is already HTML."),
    ("Medium", "Settings → Account → Download your information. The archive holds one HTML file per post."),
    ("Squarespace", "Settings → Import & Export → Export, which produces a WordPress-format file."),
];

pub(crate) async fn migration_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let group = SETTINGS_GROUPS.iter().find(|g| g.slug == "migration").unwrap();

    // What the archive would actually contain, so the checkbox list can say
    // how much there is rather than only what there is.
    let member_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM members").fetch_one(&state.db).await.unwrap_or(0);
    let post_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM posts WHERE type = 'post'")
        .fetch_one(&state.db)
        .await
        .unwrap_or(0);
    let page_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM posts WHERE type = 'page'")
        .fetch_one(&state.db)
        .await
        .unwrap_or(0);
    let theme_count = ghost_api::themes::list(&state).len();

    let components: String = EXPORT_COMPONENTS
        .iter()
        .map(|(key, label, detail, default_on)| {
            format!(
                r#"<label class="fchip exp"><input type="checkbox" name="{key}" value="1"{on}><span class="ck">{check}</span>
<span class="expl"><b>{label}</b><span>{detail}</span></span></label>"#,
                key = esc(key),
                on = if *default_on { " checked" } else { "" },
                check = svg("check"),
                label = esc(label),
                detail = esc(detail),
            )
        })
        .collect();

    let sources: String = MIGRATION_SOURCES
        .iter()
        .map(|(platform, how)| {
            format!(
                r#"<div class="row" style="align-items:flex-start"><span class="ic tint-blue">{ic}</span>
<div class="body"><span class="title">{platform}</span><span class="detail">{how}</span></div></div>"#,
                ic = svg("globe"),
                platform = esc(platform),
                how = esc(how),
            )
        })
        .collect();

    page_for_role(
        actor_role,
        "Migration tools",
        "settings",
        Some(&user),
        &format!(
            r#"{head}{banner}
<div class="mtabs">
<input type="radio" name="mtab" id="mtab-import" checked>
<input type="radio" name="mtab" id="mtab-export">
<div class="mtab-bar"><label for="mtab-import">Import</label><label for="mtab-export">Export</label></div>

<div class="mtab-panel" data-tab="import">
<div class="section">Universal import</div>
<div class="card padded rise" style="--i:1">
<form method="post" action="/ghost/settings/migration/import/" enctype="multipart/form-data" class="inline-form" style="align-items:center">
<input type="file" name="file" accept=".json,.zip,application/json,application/zip" required class="filefield">
<button class="btn tonal" type="submit">{upload} Import</button>
</form>
<p class="muted" style="margin:12px 0 0">A Ghost export — either the <code>.json</code> file or a <code>.zip</code> holding one.
A zip's <code>content/images</code> and <code>content/files</code> are restored alongside the content, so posts keep their pictures.</p>
<p class="muted" style="margin:8px 0 0">Posts, pages and tags are added; Markdown is re-rendered on the way in. Nothing already on this
site is replaced or removed, so importing the same file twice gives you two copies rather than one updated one.</p>
</div>
<div class="section">Members</div>
<div class="card rows rise" style="--i:2">
<div class="row"><span class="ic tint-blue">{people}</span>
<div class="body"><span class="title">Import members from CSV</span><span class="detail">Map the columns of any CSV onto member fields, labels and tiers</span></div>
<div class="trailing"><a class="btn tonal small" href="/ghost/members/">Go to Members</a></div></div>
</div>
<div class="section">Coming from another platform</div>
<div class="card rows rise" style="--i:3">{sources}</div>
<p class="muted" style="margin:10px 2px 0">Ghost(Pro) runs a hosted migration service for these. This is a self-hosted site, so
the route is the same but manual: export from the old platform, then feed the file to the importer above.</p>
</div>

<div class="mtab-panel" data-tab="export">
<div class="section">Export all data</div>
<form class="card padded rise" style="--i:1" method="get" action="/ghost/settings/migration/export/">
<p class="muted" style="margin:0 0 14px">Pick what to put in the archive. This is the one to take before an upgrade, or when moving the site somewhere else.</p>
<div class="expgrid">{components}</div>
<div style="margin-top:16px"><button class="btn" type="submit">{download} Download archive</button></div>
</form>
<div class="section">Individual files</div>
<div class="card rows rise" style="--i:2">
<div class="row"><span class="ic tint-blue">{notes}</span>
<div class="body"><span class="title">Content &amp; settings</span><span class="detail">{posts} posts and {pages} pages, with their tags · JSON</span></div>
<div class="trailing"><a class="btn tonal small" href="/ghost/settings/migration/export/content/">Download</a></div></div>
<div class="row"><span class="ic tint-blue">{people}</span>
<div class="body"><span class="title">Members</span><span class="detail">{members} members with labels, tiers and Stripe ids · CSV</span></div>
<div class="trailing"><a class="btn tonal small" href="/ghost/members/export/">Download</a></div></div>
<div class="row"><span class="ic tint-blue">{chart}</span>
<div class="body"><span class="title">Post analytics</span><span class="detail">Sends, opens, views and feedback, one row per post · CSV</span></div>
<div class="trailing"><a class="btn tonal small" href="/ghost/settings/migration/export/analytics/">Download</a></div></div>
<div class="row"><span class="ic tint-blue">{themes_ic}</span>
<div class="body"><span class="title">Themes</span><span class="detail">{themes} installed, including anything edited in the browser · ZIP</span></div>
<div class="trailing"><a class="btn tonal small" href="/ghost/settings/migration/export/themes/">Download</a></div></div>
<div class="row"><span class="ic tint-blue">{link}</span>
<div class="body"><span class="title">Redirects &amp; routes</span><span class="detail">The two YAML files, downloadable one at a time</span></div>
<div class="trailing"><a class="btn tonal small" href="/ghost/settings/advanced/">Go to Advanced</a></div></div>
</div>
<p class="muted" style="margin:10px 2px 0">An export is a copy, not a backup of the database itself — it holds your content, members and
files, not sessions, API keys or password hashes.</p>
</div>
</div>"#,
            head = group_head(group, None),
            banner = flash_banner(&q),
            upload = svg("upload"),
            download = svg("import"),
            people = svg("person"),
            notes = svg("notes"),
            chart = svg("chart"),
            themes_ic = svg("themes"),
            link = svg("link"),
            sources = sources,
            components = components,
            posts = post_count,
            pages = page_count,
            members = member_count,
            themes = theme_count,
        ),
        true,
    )
    .into_response()
}

// --- code injection: a page of its own under Settings ------------------

pub(crate) async fn code_injection_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let s = |k: &str| esc(&state.settings.get_str(k).unwrap_or_default());
    let group = SETTINGS_GROUPS.iter().find(|g| g.slug == "code-injection").unwrap();
    page_for_role(actor_role,
        "Code injection",
        "settings",
        Some(&user),
        &format!(
            r#"{head}{banner}
<form method="post" action="/ghost/settings/code-injection/" id="codeinjection-form">
<div class="section">Site head</div>
<div class="card rows rise" style="--i:1">
<div class="row" style="align-items:flex-start"><span class="ic tint-purple">{code}</span>
<div class="body" style="width:100%"><label class="field" style="margin:0"><textarea name="codeinjection_head" class="code" rows="8" placeholder=" ">{cih}</textarea><span>Injected into &lt;head&gt; on every page</span></label></div></div>
</div>
<div class="section">Site foot</div>
<div class="card rows rise" style="--i:2">
<div class="row" style="align-items:flex-start"><span class="ic tint-purple">{code}</span>
<div class="body" style="width:100%"><label class="field" style="margin:0"><textarea name="codeinjection_foot" class="code" rows="8" placeholder=" ">{cif}</textarea><span>Injected before &lt;/body&gt; on every page</span></label></div></div>
</div>
</form>
<p class="muted" style="margin:10px 2px 0">Styles, scripts or analytics for the whole site — pasted as-is, so tags and quotes
stay exactly as written.</p>"#,
            head = group_head(group, Some("codeinjection-form")),
            banner = flash_banner(&q),
            code = svg("code"),
            cih = s("codeinjection_head"),
            cif = s("codeinjection_foot"),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn code_injection_save(State(state): State<AppState>, headers: HeaderMap, Form(form): Form<HashMap<String, String>>) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let mut ok = true;
    for key in ["codeinjection_head", "codeinjection_foot"] {
        let value = form.get(key).cloned().unwrap_or_default();
        ok &= state.settings.set(&state.db, key, &serde_json::json!(value)).await.is_ok();
    }
    if ok {
        settings_saved(&state, "code-injection", "Code injection saved.")
    } else {
        settings_flash("code-injection", "error", "Saving failed — check the logs.")
    }
}

/// Save the feature flags, stored as keys on the `labs` settings object —
/// the same object Ghost writes, so a flag set here is the flag a Ghost
/// theme or export expects.
///
/// Unchecked switches post nothing, so each rendered row carries a `shown`
/// marker and only those are rewritten — a flag whose row was not on the page
/// (the private ones outside development) keeps whatever it held, and a flag
/// set by a newer Ghost that this build has no row for survives untouched.
// --- export / import (Ghost-format JSON) ---

/// Ghost export shape: `{db: [{meta: {…}, data: {posts, tags, users, …}}]}`.
/// Kept as a pure function over the fetched rows so it can be unit-tested.
#[cfg(test)]
pub(crate) fn build_ghost_export(posts: &[ghost_core::models::Post], tags: &[ghost_core::models::Tag], site_url: &str) -> serde_json::Value {
    let post_rows: Vec<serde_json::Value> = posts
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "uuid": p.uuid,
                "title": p.title,
                "slug": p.slug,
                "markdown": p.markdown,
                "html": p.html,
                "plaintext": p.plaintext,
                "feature_image": p.feature_image,
                "featured": p.featured,
                "type": p.post_type,
                "status": p.status,
                "visibility": p.visibility,
                "custom_excerpt": p.custom_excerpt,
                "reading_time": p.reading_time,
                "created_at": p.created_at,
                "updated_at": p.updated_at,
                "published_at": p.published_at,
            })
        })
        .collect();
    let tag_rows: Vec<serde_json::Value> = tags
        .iter()
        .map(|t| {
            serde_json::json!({
                "id": t.id,
                "name": t.name,
                "slug": t.slug,
                "description": t.description,
                "visibility": t.visibility,
            })
        })
        .collect();
    serde_json::json!({
        "db": [{
            "meta": {
                "exported_on": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
                "version": GHOST_VERSION,
            },
            "data": {
                "posts": post_rows,
                "tags": tag_rows,
                "site_url": site_url,
            }
        }]
    })
}

/// The content export as a JSON string: posts, pages and tags in Ghost's
/// export shape. Shared by the standalone download and the backup bundle.
pub(crate) async fn build_content_export(state: &AppState) -> String {
    build_content_export_with(state, &[]).await
}

/// The content export, optionally carrying extended backup tables.
///
/// `include` names tables from `transfer::BACKUP_TABLES` — members and their
/// whole event history, emails, comments, automations, sessions and keys. The
/// default export is deliberately without them: it is the portable one, meant
/// to be moved between installations, and it is the one a publisher is most
/// likely to hand to somebody else. A complete backup is a different thing
/// and has to be asked for.
pub(crate) async fn build_content_export_with(state: &AppState, include: &[String]) -> String {
    match ghost_db::transfer::export_with(&state.db, include).await {
        Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_default(),
        Err(error) => { tracing::error!("Content export failed: {}", error.message()); String::new() }
    }
}

pub(crate) async fn migration_export_content(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json; charset=utf-8".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"ghost-export.{}.json\"", export_date_stamp()),
            ),
        ],
        build_content_export(&state).await,
    )
        .into_response()
}

/// Post analytics from native page views, newsletter clicks and attributed
/// membership/subscription events. These CSV fields share the report sources.
pub(crate) const POST_ANALYTICS_COLUMNS: &[&str] = &[
    "id", "title", "url", "author", "status", "created_at", "updated_at", "published_at",
    "featured", "tags", "post_access", "email_recipients", "newsletter_name", "sends", "opens",
    "views", "feedback_more_like_this", "feedback_less_like_this", "clicks", "signups", "paid_conversions",
];

/// Per-post sends, opens, views and feedback, as a CSV.
pub(crate) async fn build_post_analytics_csv(state: &AppState) -> Option<Vec<u8>> {
    let q = repos::posts::BrowsePosts {
        filter: None,
        order: Some("published_at desc".into()),
        limit: 100_000,
        page: 1,
        post_type: Some("post".into()),
        public: false,
        include_authors: true,
        include_tags: true,
    };
    let posts = repos::posts::browse(&state.db, &q).await.map(|p| p.posts).unwrap_or_default();
    let site_url = state.site_url();
    let site_url = site_url.trim_end_matches('/');

    let mut w = csv::Writer::from_writer(Vec::new());
    w.write_record(POST_ANALYTICS_COLUMNS).ok()?;
    for p in &posts {
        let slug = p.slug.clone().unwrap_or_default();
        // The email for a post, if one went out. A post reverted to draft
        // keeps its email row, so the status check below is what stops a
        // draft reporting last month's sends.
        let email = sqlx::query_as::<_, (i64, i64, Option<ghost_db::types::Text>, Option<String>)>(
            "SELECT email_count, opened_count, recipient_filter, newsletter_id \
             FROM emails WHERE post_id = ? LIMIT 1",
        )
        .bind(&p.id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
        .filter(|_| p.status == "published" || p.status == "sent");

        let newsletter_name = match email.as_ref().and_then(|e| e.3.clone()) {
            Some(nid) => sqlx::query_scalar::<_, String>("SELECT name FROM newsletters WHERE id = ?")
                .bind(&nid)
                .fetch_optional(&state.db)
                .await
                .ok()
                .flatten()
                .unwrap_or_default(),
            None => String::new(),
        };

        // Views are counted against the rendered path, which is the slug.
        let views: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM page_views WHERE path = ? OR path = ?")
            .bind(format!("/{slug}/"))
            .bind(format!("/{slug}"))
            .fetch_one(&state.db)
            .await
            .unwrap_or(0);

        let feedback = |score: i64| async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM members_feedback WHERE post_id = ? AND score = ?",
            )
            .bind(&p.id)
            .bind(score)
            .fetch_one(&state.db)
            .await
            .unwrap_or(0)
        };
        let (more, less) = (feedback(1).await, feedback(0).await);

        let clicks:i64=sqlx::query_scalar("SELECT COUNT(*) FROM newsletter_clicks c JOIN newsletter_links l ON l.id=c.link_id WHERE l.post_id=?").bind(&p.id).fetch_one(&state.db).await.unwrap_or(0);
        let signups:i64=sqlx::query_scalar("SELECT COUNT(*) FROM members_created_events WHERE attribution_id=? AND attribution_type='post'").bind(&p.id).fetch_one(&state.db).await.unwrap_or(0);
        let conversions:i64=sqlx::query_scalar("SELECT COUNT(*) FROM members_subscription_created_events WHERE attribution_id=? AND attribution_type='post'").bind(&p.id).fetch_one(&state.db).await.unwrap_or(0);
        let record = [
            p.id.clone(),
            p.title.clone().unwrap_or_default(),
            format!("{site_url}/{slug}/"),
            post_author_names(state, &p.id).await,
            p.status.clone(),
            p.created_at.clone().unwrap_or_default(),
            p.updated_at.clone().unwrap_or_default(),
            p.published_at.clone().unwrap_or_default(),
            if p.featured { "true".into() } else { "false".into() },
            post_tag_names(state, &p.id).await,
            p.visibility.clone(),
            // `emails.recipient_filter` is MEDIUMTEXT on MySQL, hence `Text`.
            email.as_ref().and_then(|e| e.2.clone()).map(|t| t.0).unwrap_or_default(),
            newsletter_name,
            email.as_ref().map(|e| e.0.to_string()).unwrap_or_default(),
            email.as_ref().map(|e| e.1.to_string()).unwrap_or_default(),
            views.to_string(),
            more.to_string(),
            less.to_string(),
            clicks.to_string(), signups.to_string(), conversions.to_string(),
        ];
        w.write_record(&record).ok()?;
    }
    w.into_inner().ok()
}

/// The names behind a post's authors and tags, comma-joined as Ghost writes
/// them. Queried per post rather than taken from the browse include, because
/// the join only populates when the caller asked for it and a missing name
/// would silently export an empty column.
pub(crate) async fn post_author_names(state: &AppState, post_id: &str) -> String {
    sqlx::query_scalar::<_, String>(
        "SELECT u.name FROM users u JOIN posts_authors pa ON pa.author_id = u.id \
         WHERE pa.post_id = ? ORDER BY pa.sort_order",
    )
    .bind(post_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default()
    .join(", ")
}

pub(crate) async fn post_tag_names(state: &AppState, post_id: &str) -> String {
    sqlx::query_scalar::<_, String>(
        "SELECT t.name FROM tags t JOIN posts_tags pt ON pt.tag_id = t.id \
         WHERE pt.post_id = ? ORDER BY pt.sort_order",
    )
    .bind(post_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default()
    .join(", ")
}

pub(crate) async fn migration_export_analytics(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    match build_post_analytics_csv(&state).await {
        Some(body) => csv_download(&format!("posts.analytics.{}.csv", export_date_stamp()), body),
        None => settings_flash("migration", "error", "Could not build the analytics export."),
    }
}

// --- the export bundle -------------------------------------------------------

/// The pieces an export may carry, in the order they are offered.
///
/// `(key, label, detail, on by default)`. Media is off by default for the
/// reason Ghost gives: it is usually the bulk of the archive, and someone
/// moving content between sites rarely wants to wait for it.
/// Build a queued export in the background, then tell the requester.
///
/// Detached from the request on purpose: a full export with media in it can
/// take minutes, which is why `POST /exports` answers 202 rather than holding
/// the connection. The row is the only record of what happened, so it is
/// updated whether the build succeeds or fails — an export that quietly never
/// finishes is indistinguishable from one still running.
pub(crate) fn spawn_export_build(state: AppState, id: String, components: serde_json::Value) {
    tokio::spawn(async move {
        let want = |key: &str| components.get(key) == Some(&serde_json::json!(true));
        let _ = sqlx::query("UPDATE export_requests SET status = 'running', started_at = ? WHERE id = ?")
            .bind(ghost_core::dates::now_db())
            .bind(&id)
            .execute(&state.db)
            .await;

        let slug = ghost_core::slug::safe_string(&state.settings.site_name());
        let prefix = if slug.is_empty() { String::new() } else { format!("{slug}.") };
        // The id is in the name so two exports of the same site on the same
        // day cannot overwrite one another.
        let filename = format!("{prefix}ghost.export.{}.{id}.zip", export_date_stamp());
        let path = state.config.content_path().join("exports").join(&filename);

        match build_export_archive_to_file(&state, &want, &path).await {
            Some(size) => {
                let _ = sqlx::query(
                    "UPDATE export_requests SET status = 'finished', filename = ?, size_bytes = ?,                      finished_at = ?, error = NULL WHERE id = ?",
                )
                .bind(&filename)
                .bind(size as i64)
                .bind(ghost_core::dates::now_db())
                .bind(&id)
                .execute(&state.db)
                .await;
                notify_export_ready(&state, &id, &filename, size).await;
            }
            None => {
                let _ = sqlx::query(
                    "UPDATE export_requests SET status = 'failed', error = ?, finished_at = ? WHERE id = ?",
                )
                .bind("The archive could not be written. Check disk space and content/ permissions.")
                .bind(ghost_core::dates::now_db())
                .bind(&id)
                .execute(&state.db)
                .await;
                tracing::error!("export {id} failed to build");
            }
        }
    });
}

/// Email the staff user who asked for an export that it is ready.
///
/// Upstream's host service does this; here it is the only notification a
/// requester gets, since nothing was holding their connection open. A site
/// with no mailer configured simply leaves the row for `GET /exports` to
/// report — the archive is still there either way.
pub(crate) async fn notify_export_ready(state: &AppState, id: &str, filename: &str, size: u64) {
    let requester: Option<(Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT u.name, u.email FROM export_requests e JOIN users u ON u.id = e.requested_by          WHERE e.id = ?",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let Some((name, Some(email))) = requester else { return };
    let title = state.settings.site_name();
    let site = state.site_url();
    let link = format!("{}/ghost/api/admin/exports/{id}/download/", site.trim_end_matches('/'));
    let megabytes = (size as f64) / (1024.0 * 1024.0);
    let html = format!(
        r#"<p>Hi {},</p><p>The export of {} you asked for is ready.</p>
<p><a href="{}">Download {} ({:.1} MB)</a></p>
<p>You will need to be signed in to the admin on this device to collect it.</p>"#,
        esc(name.as_deref().unwrap_or("there")),
        esc(&title),
        esc(&link),
        esc(filename),
        megabytes,
    );
    let cfg = ghost_mail::MailConfig::from_settings(&state.settings, &title, &site);
    let msg = ghost_mail::Message {
        to: email,
        subject: format!("Your {title} export is ready"),
        html,
        text: format!("The export of {title} you asked for is ready.

{link}

{filename} ({megabytes:.1} MB)"),
    };
    if let Err(e) = ghost_mail::send(&cfg, &msg).await {
        tracing::warn!("export-ready email failed: {}", e.message());
    }
}

pub(crate) const EXPORT_COMPONENTS: &[(&str, &str, &str, bool)] = &[
    ("content", "Content & settings", "Posts, pages and tags, in Ghost's JSON export format", true),
    ("members", "Members", "Every member with their labels and tiers (CSV)", true),
    ("analytics", "Post analytics", "Sends, opens, views and feedback per post (CSV)", true),
    ("themes", "Themes", "Every installed theme, including anything you edited", true),
    ("routes", "Redirects & routes", "routes.yaml and redirects.yaml as they stand", true),
    (
        "media",
        "Media files",
        "Every uploaded image and file. Can make the archive very much larger",
        false,
    ),
    (
        "state",
        "Members & full history",
        "Members, emails, comments, automations, events and keys — a restore, not a content move. Keep it secret",
        false,
    ),
];

/// Add a directory to the archive, recursively, under `prefix`.
///
/// Walked breadth-first with an explicit stack rather than by recursion
/// because `ZipWriter` is borrowed mutably throughout, and an async recursive
/// helper around it would need boxing for no gain.
pub(crate) fn zip_add_dir<W: std::io::Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    base: &std::path::Path,
    prefix: &str,
    opts: zip::write::SimpleFileOptions,
) -> std::io::Result<()> {
    let mut stack = vec![base.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            // Dotfiles and dependency trees are never part of a backup.
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            if entry.file_type()?.is_symlink() {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Ok(rel) = path.strip_prefix(base) else { continue };
            let Ok(mut file) = std::fs::File::open(&path) else { continue };
            zip.start_file(format!("{prefix}/{}", rel.to_string_lossy().replace('\\', "/")), opts)?;
            // Copied rather than read whole: a video in content/media can be
            // larger than the process should ever hold at once, and the point
            // of writing the archive to disk is undone by buffering its
            // inputs in memory.
            std::io::copy(&mut file, zip)?;
        }
    }
    Ok(())
}

/// Build the export archive from the components the operator ticked.
///
/// Kept for the small archives — a content-only or themes-only download,
/// where holding it in memory costs nothing and saves a temporary file.
/// Anything that can include media goes through
/// [`build_export_archive_to_file`] instead.
pub(crate) async fn build_export_archive(
    state: &AppState,
    want: &(dyn Fn(&str) -> bool + Send + Sync),
) -> Option<Vec<u8>> {
    let mut buf = std::io::Cursor::new(Vec::new());
    write_export_archive(state, want, &mut buf).await?;
    Some(buf.into_inner())
}

/// Assemble an archive on disk, and return its path and size.
///
/// This is the path for a full backup. The previous implementation built
/// every archive in a `Vec<u8>`, so exporting a site's media meant holding
/// all of it in memory at once — on a publication with a few gigabytes of
/// video that is not a slow export, it is a dead process. Writing through a
/// file keeps the peak cost at one zip window regardless of site size, and
/// the response then streams the file back in chunks.
///
/// The caller owns the file and is responsible for removing it.
pub(crate) async fn build_export_archive_to_file(
    state: &AppState,
    want: &(dyn Fn(&str) -> bool + Send + Sync),
    path: &std::path::Path,
) -> Option<u64> {
    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let file = std::fs::File::create(path).ok()?;
    let mut file = std::io::BufWriter::new(file);
    let written = write_export_archive(state, want, &mut file).await;
    // Flush before measuring, or the size is whatever happened to have left
    // the buffer.
    let flushed = std::io::Write::flush(&mut file).is_ok();
    drop(file);
    if written.is_none() || !flushed {
        let _ = std::fs::remove_file(path);
        return None;
    }
    std::fs::metadata(path).ok().map(|m| m.len())
}

/// The archive itself, written into whatever the caller provides.
pub(crate) async fn write_export_archive<W: std::io::Write + std::io::Seek>(
    state: &AppState,
    want: &(dyn Fn(&str) -> bool + Send + Sync),
    sink: &mut W,
) -> Option<()> {
    use std::io::Write;
    {
        let mut zip = zip::ZipWriter::new(sink);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        if want("content") {
            zip.start_file("export.json", opts).ok()?;
            // `state` is the extended-backup component: with it ticked the
            // export.json carries the operational tables too, so the archive
            // is a restore rather than a content move.
            let include: Vec<String> = if want("state") {
                ghost_db::transfer::BACKUP_TABLES.iter().map(|t| (*t).to_string()).collect()
            } else {
                Vec::new()
            };
            zip.write_all(build_content_export_with(state, &include).await.as_bytes()).ok()?;
        }
        if want("members") {
            if let Some(csv) = build_members_csv(state, None).await {
                zip.start_file("members.csv", opts).ok()?;
                zip.write_all(&csv).ok()?;
            }
        }
        if want("analytics") {
            if let Some(csv) = build_post_analytics_csv(state).await {
                zip.start_file("post-analytics.csv", opts).ok()?;
                zip.write_all(&csv).ok()?;
            }
        }
        if want("themes") {
            zip_add_dir(&mut zip, &state.config.themes_dir(), "themes", opts).ok()?;
        }
        if want("routes") {
            // At the archive root, where Ghost writes them, so an export
            // from here restores into Ghost and not only back into this port.
            for file in ["routes.yaml", "redirects.yaml"] {
                if let Ok(bytes) = std::fs::read(state.config.content_path().join(file)) {
                    zip.start_file(file, opts).ok()?;
                    zip.write_all(&bytes).ok()?;
                }
            }
        }
        if want("media") {
            for (dir,prefix) in archive_media_paths(&state.config) {
                zip_add_dir(&mut zip, &dir, prefix, opts).ok()?;
            }
        }
        zip.finish().ok()?;
    }
    Some(())
}

pub(crate) fn zip_download(filename: &str, body: Vec<u8>) -> Response {
    (
        [
            (header::CONTENT_TYPE, "application/zip".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{filename}\"")),
        ],
        body,
    )
        .into_response()
}

/// `?content=1&members=1&…` — one archive of whatever was ticked.
pub(crate) async fn migration_export_all(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let want = |key: &str| q.get(key).map(|v| v == "1").unwrap_or(false);
    if !EXPORT_COMPONENTS.iter().any(|(k, _, _, _)| want(k)) {
        return settings_flash("migration", "error", "Pick at least one thing to export.");
    }
    match build_export_archive(&state, &want).await {
        Some(body) => {
            zip_download(&format!("ghost-backup.{}.zip", export_date_stamp()), body)
        }
        None => settings_flash("migration", "error", "Could not build the archive."),
    }
}

pub(crate) async fn migration_export_themes(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let only_themes = |key: &str| key == "themes";
    match build_export_archive(&state, &only_themes).await {
        Some(body) => zip_download(&format!("themes.{}.zip", export_date_stamp()), body),
        None => settings_flash("migration", "error", "Could not build the themes archive."),
    }
}

/// Result of walking an import file, reported back through the flash.
#[derive(Default)]
pub(crate) struct ImportCounts {
    pub(crate) posts: usize,
    pub(crate) pages: usize,
    pub(crate) tags: usize,
}

impl ImportCounts {
    pub(crate) fn message(&self) -> String {
        format!(
            "Imported {} {}, {} {} and {}.",
            self.posts,
            if self.posts == 1 { "post" } else { "posts" },
            self.tags,
            if self.tags == 1 { "tag" } else { "tags" },
            if self.pages == 1 { "1 page".to_string() } else { format!("{} pages", self.pages) },
        )
    }
}

/// Insert posts/pages/tags from a Ghost export file's `data` object.
pub(crate) async fn run_import(state: &AppState, owner_id: &str, data: &serde_json::Value) -> Result<ImportCounts, ghost_core::GhostError> {
    let prepared=ghost_api::publishing::prepare_content_import(data);
    ghost_db::transfer::import(&state.db, &prepared, owner_id).await?;
    state.settings.reload(&state.db).await?;
    let posts = data["posts"].as_array().cloned().unwrap_or_default();
    Ok(ImportCounts {
        posts: posts.iter().filter(|p|p["type"] != "page").count(),
        pages: posts.iter().filter(|p|p["type"] == "page").count(),
        tags: data["tags"].as_array().map_or(0, Vec::len),
    })
}

/// What came out of an uploaded archive.
pub(crate) struct ArchiveContents {
    /// The Ghost export JSON found inside, if there was one.
    export: Option<Vec<u8>>,
    /// Media restored to `content/`, counted for the report.
    media_restored: usize,
}

pub(crate) fn archive_media_paths(config: &ghost_core::config::Config) -> [(std::path::PathBuf, &'static str);3] {
    [(config.images_dir(),"content/images"), (config.files_dir(),"content/files"), (config.media_dir(),"content/media")]
}

pub(crate) fn archive_media_target(config: &ghost_core::config::Config, path: &str) -> Option<std::path::PathBuf> {
    archive_media_paths(config).into_iter().find_map(|(dir,prefix)| {
        after_segment(path,&format!("{prefix}/")).map(|rest|dir.join(rest))
    })
}

/// Unpack an uploaded zip: find the export JSON, restore any media beside it.
///
/// Both shapes are accepted — the archive this screen produces
/// (`ghost-export.json` + `content/images/…`) and Ghost's own, which nests
/// everything one level down inside a named folder. Rather than guess at the
/// layout, the first `.json` that parses as an export wins and any path under
/// a `content/images`, `content/files` or `content/media` segment is treated as media.
///
/// Entry paths are the untrusted part: a zip may name `../../etc/passwd`, so
/// every restored path is rebuilt from its own segments and anything with a
/// traversal or an absolute root is skipped rather than sanitised.
pub(crate) fn unpack_import_archive(
    state: &AppState,
    data: &[u8],
) -> Result<ArchiveContents, ghost_core::GhostError> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(data))
        .map_err(|e| ghost_core::GhostError::BadRequest(format!("Not a readable zip: {e}")))?;
    let mut export = None;
    let mut media_restored = 0;

    for i in 0..archive.len() {
        let Ok(mut entry) = archive.by_index(i) else { continue };
        if entry.is_dir() {
            continue;
        }
        // `enclosed_name` is the zip crate's own traversal check: it returns
        // None for an absolute path or one that climbs out of the archive.
        let Some(path) = entry.enclosed_name() else { continue };
        let rel = path.to_string_lossy().replace('\\', "/");
        let mut bytes = Vec::new();
        if std::io::Read::read_to_end(&mut entry, &mut bytes).is_err() {
            continue;
        }

        if export.is_none() && rel.to_lowercase().ends_with(".json") {
            // A theme's package.json is also a .json; only something shaped
            // like an export counts.
            if serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .and_then(|v| v.get("db").cloned())
                .is_some()
            {
                export = Some(bytes);
                continue;
            }
        }

        let target = archive_media_target(&state.config,&rel);
        if let Some(target) = target {
            if let Some(parent) = target.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::write(&target, &bytes).is_ok() {
                media_restored += 1;
            }
        }
    }
    Ok(ArchiveContents { export, media_restored })
}

/// The part of `path` after `marker`, if the marker is a whole-segment match.
///
/// Used to find media inside an archive whatever it is nested under, without
/// matching a directory that merely ends in the same letters.
pub(crate) fn after_segment(path: &str, marker: &str) -> Option<String> {
    let at = path.find(marker)?;
    if at != 0 && !path[..at].ends_with('/') {
        return None;
    }
    let rest = &path[at + marker.len()..];
    // Rebuild from segments so nothing odd survives into a filesystem path.
    if rest.is_empty() || rest.split('/').any(|s| s.is_empty() || s == "." || s == "..") {
        return None;
    }
    Some(rest.to_string())
}

/// The universal importer: a Ghost export as JSON, or a zip holding one.
pub(crate) async fn migration_import(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    let (user, _actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    while let Ok(Some(field)) = multipart.next_field().await {
        if !matches!(field.name(), Some("file" | "importfile")) {
            continue;
        }
        let filename = field.file_name().unwrap_or_default().to_ascii_lowercase();
        let data = match field.bytes().await {
            Ok(d) => d.to_vec(),
            Err(e) => return flash(&headers, "migration", "error", &format!("Upload failed: {e}")),
        };
        if data.is_empty() {
            return flash(&headers, "migration", "error", "That file is empty.");
        }

        // A zip starts "PK\x03\x04". Sniffing the bytes rather than trusting
        // the extension means a correctly-named file that is not a zip fails
        // with "not valid JSON" instead of a confusing zip error.
        let is_zip = data.starts_with(b"PK\x03\x04") || filename.ends_with(".zip");
        let (json, media_note) = if is_zip {
            match unpack_import_archive(&state, &data) {
                Ok(contents) => {
                    let note = match contents.media_restored {
                        0 => String::new(),
                        1 => " 1 media file was restored.".to_string(),
                        n => format!(" {n} media files were restored."),
                    };
                    match contents.export {
                        Some(bytes) => (bytes, note),
                        None if contents.media_restored > 0 => {
                            return flash(&headers, 
                                "migration",
                                "ok",
                                &format!("No export file in that archive, but{note}"),
                            )
                        }
                        None => {
                            return flash(&headers, 
                                "migration",
                                "error",
                                "That archive holds no Ghost export file and no media.",
                            )
                        }
                    }
                }
                Err(e) => return flash(&headers, "migration", "error", &e.message()),
            }
        } else {
            (data, String::new())
        };

        let parsed: serde_json::Value = match serde_json::from_slice(&json) {
            Ok(v) => v,
            Err(e) => {
                return flash(&headers, "migration", "error", &format!("Not a valid JSON file: {e}"))
            }
        };
        let data = parsed
            .get("db")
            .and_then(|d| d.as_array())
            .and_then(|a| a.first())
            .and_then(|entry| entry.get("data"));
        return match data {
            Some(data) => match run_import(&state, &user.id, data).await {
                Ok(counts) => {
                    flash(&headers, "migration", "ok", &format!("{}{media_note}", counts.message()))
                }
                Err(e) => flash(&headers, "migration", "error", &e.message()),
            },
            None => flash(&headers, 
                "migration",
                "error",
                "That file doesn't look like a Ghost export (no db[0].data).",
            ),
        };
    }
    flash(&headers, "migration", "error", "No file was uploaded.")
}

// --- labs: the redirects and routes files, uploadable and downloadable ---

/// The current redirects.yaml, as Ghost's Labs screen offers it. A site that
/// has never uploaded any gets the neutral document rather than an error.
pub(crate) async fn redirects_download(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let body = tokio::fs::read_to_string(state.config.content_path().join("redirects.yaml"))
        .await
        .unwrap_or_else(|_| "# No redirects configured yet.\n".to_string());
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/yaml; charset=utf-8"),
            (header::CONTENT_DISPOSITION, "attachment; filename=\"redirects.yaml\""),
        ],
        body,
    )
        .into_response()
}

pub(crate) async fn redirects_upload(State(state): State<AppState>, headers: HeaderMap, mut multipart: Multipart) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    while let Ok(Some(field)) = multipart.next_field().await {
        if !matches!(field.name(), Some("file" | "redirects")) {
            continue;
        }
        let text = match field.text().await {
            Ok(t) => t,
            Err(e) => return flash(&headers, "advanced", "error", &format!("Upload failed: {e}")),
        };
        // Parse before writing anything: a malformed file never reaches the
        // disk, let alone the live redirect table.
        let parsed = match ghost_routing::redirects::parse_redirects_yaml(&text) {
            Ok(r) => r,
            Err(e) => return flash(&headers, "advanced", "error", &format!("Not a valid redirects file: {e}")),
        };
        let path = state.config.content_path().join("redirects.yaml");
        if let Err(e) = tokio::fs::write(&path, &text).await {
            return flash(&headers, "advanced", "error", &format!("Could not write redirects.yaml: {e}"));
        }
        let count = ghost_routing::redirects::rules_map(&parsed).len();
        *state.redirects.write().await = Arc::new(parsed);
        fire_site_changed(&state);
        return flash(&headers, "advanced", "ok", &format!("Redirects uploaded — {count} rule{s} in force.", s = if count == 1 { "" } else { "s" }));
    }
    flash(&headers, "advanced", "error", "No file was uploaded.")
}

pub(crate) async fn routes_download(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let body = tokio::fs::read_to_string(state.config.content_path().join("routes.yaml"))
        .await
        .unwrap_or_else(|_| ghost_routing::DEFAULT_ROUTES_YAML.to_string());
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/yaml; charset=utf-8"),
            (header::CONTENT_DISPOSITION, "attachment; filename=\"routes.yaml\""),
        ],
        body,
    )
        .into_response()
}

/// Upload a routes.yaml. Routing is live configuration, so the file is
/// parsed *and* built into a router tree before anything is written — then
/// the tree is swapped in and the active theme is recompiled against it,
/// because a theme's resolvers and template choices are derived from routes.
pub(crate) async fn routes_upload(State(state): State<AppState>, headers: HeaderMap, mut multipart: Multipart) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    while let Ok(Some(field)) = multipart.next_field().await {
        if !matches!(field.name(), Some("file" | "routes")) {
            continue;
        }
        let text = match field.text().await {
            Ok(t) => t,
            Err(e) => return flash(&headers, "advanced", "error", &format!("Upload failed: {e}")),
        };
        let settings = match ghost_routing::routes_yaml::parse_routes_yaml(&text) {
            Ok(s) => s,
            Err(e) => return flash(&headers, "advanced", "error", &format!("Not a valid routes file: {e}")),
        };
        let tree = match ghost_routing::RouterTree::build(&settings) {
            Ok(t) => t,
            Err(e) => return flash(&headers, "advanced", "error", &format!("These routes cannot be routed: {e}")),
        };
        let path = state.config.content_path().join("routes.yaml");
        if let Err(e) = tokio::fs::write(&path, &text).await {
            return flash(&headers, "advanced", "error", &format!("Could not write routes.yaml: {e}"));
        }
        *state.router.write().await = Arc::new(tree);
        // The theme was compiled against the old routes; recompile it against
        // the new ones. A failure here leaves routing updated but the old
        // theme in place — said out loud rather than half-reported.
        let theme = ghost_api::themes::active_theme_name(&state);
        if let Err(e) = ghost_api::themes::activate(&state, &theme).await {
            return flash(&headers, 
                "advanced",
                "error",
                &format!("Routes are in force, but the theme could not be rebuilt against them: {}", e.message()),
            );
        }
        fire_site_changed(&state);
        return flash(&headers, "advanced", "ok", "Routes uploaded — routing and the theme have been rebuilt.");
    }
    flash(&headers, "advanced", "error", "No file was uploaded.")
}

// --- history: the actions audit log ---

/// Which resource types each of Ghost's history filter groups covers. The
/// port records a few names Ghost does not, so each group names its own.
pub(crate) const HISTORY_RESOURCES: &[(&str, &str, &[&str])] = &[
    ("post", "Posts", &["post"]),
    ("page", "Pages", &["page"]),
    ("tag", "Tags", &["tag"]),
    ("tier", "Tiers & offers", &["tier", "product", "offer"]),
    ("setting", "Settings & staff", &["user", "users", "setting", "settings", "integrations", "members", "db"]),
];

pub(crate) const HISTORY_EVENTS: &[(&str, &str)] =
    &[("added", "Added"), ("edited", "Edited"), ("deleted", "Deleted"), ("published", "Published")];

pub(crate) async fn history_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    // Ghost's history filters are exclusions: everything shows until you
    // untick something. A submitted form is marked, so a first load — where
    // no box has been touched — is not read as "everything excluded".
    let filtered = q.contains_key("f");
    let shown_event = |v: &str| !filtered || q.contains_key(&format!("e_{v}"));
    let shown_resource = |k: &str| !filtered || q.contains_key(&format!("r_{k}"));

    let excluded_events: Vec<&str> =
        HISTORY_EVENTS.iter().map(|(v, _)| *v).filter(|v| !shown_event(v)).collect();
    // Expand the excluded groups to the resource names they actually cover.
    let excluded_names: Vec<&str> = HISTORY_RESOURCES
        .iter()
        .filter(|(k, _, _)| !shown_resource(k))
        .flat_map(|(_, _, names)| names.iter().copied())
        .collect();

    let mut sql = String::from(
        "SELECT a.event, a.resource_type, a.resource_id, a.context, a.created_at, COALESCE(u.name, 'System') \
         FROM actions a LEFT JOIN users u ON u.id = a.actor_id WHERE 1=1",
    );
    for _ in &excluded_events {
        sql.push_str(" AND a.event != ?");
    }
    for _ in &excluded_names {
        sql.push_str(" AND a.resource_type != ?");
    }
    sql.push_str(" ORDER BY a.created_at DESC LIMIT 100");

    let mut query =
        sqlx::query_as::<_, (String, String, Option<String>, Option<String>, String, String)>(&sql);
    for e in &excluded_events {
        query = query.bind(*e);
    }
    for r in &excluded_names {
        query = query.bind(*r);
    }

    let chip = |name: String, label: &str, on: bool| {
        format!(
            r#"<label class="fchip"><input type="checkbox" name="{name}" value="1"{ck} onchange="this.form.submit()"><span class="ck">{check}</span>{label}</label>"#,
            name = esc(&name),
            label = esc(label),
            ck = if on { " checked" } else { "" },
            check = svg("check"),
        )
    };
    let filter_bar = format!(
        r#"<form method="get" action="/ghost/settings/history/" class="card padded rise" style="--i:1">
<input type="hidden" name="f" value="1">
<span class="detail" style="display:block;margin-bottom:8px">Event</span>
<div class="chips">{events}</div>
<span class="detail" style="display:block;margin:14px 0 8px">Resource</span>
<div class="chips">{resources}</div>
<noscript><div style="margin-top:12px"><button class="btn tonal small" type="submit">Apply</button></div></noscript>
</form>"#,
        events = HISTORY_EVENTS
            .iter()
            .map(|(v, l)| chip(format!("e_{v}"), l, shown_event(v)))
            .collect::<String>(),
        resources = HISTORY_RESOURCES
            .iter()
            .map(|(k, l, _)| chip(format!("r_{k}"), l, shown_resource(k)))
            .collect::<String>(),
    );

    let mut rows = String::new();
    if let Ok(events) = query.fetch_all(&state.db).await {
        if events.is_empty() {
            rows = r#"<div class="row"><div class="body"><span class="muted">Nothing has happened yet — edits and publishes show up here.</span></div></div>"#.into();
        }
        for (event, resource_type, resource_id, _context, created_at, actor) in events {
            rows.push_str(&format!(
                r#"<div class="row"><span class="ic tint-slate">{ic}</span>
<div class="body"><span class="title">{actor} {event} {resource_type}</span><span class="detail">{when}</span></div>
<div class="trailing"><span class="chip tonal">{rid}</span></div></div>"#,
                ic = svg("history"),
                actor = esc(&actor),
                event = esc(&event),
                resource_type = esc(&resource_type),
                when = esc(&created_at),
                rid = esc(resource_id.as_deref().map(|r| &r[..8.min(r.len())]).unwrap_or("—")),
            ));
        }
    }
    let group = SETTINGS_GROUPS.iter().find(|g| g.slug == "history").unwrap();
    page_for_role(actor_role, 
        "History",
        "settings",
        Some(&user),
        &format!(
            r#"{head}
<div class="section">Filter</div>
{filter_bar}
<div class="section">Site activity</div>
<div class="card rows rise" style="--i:2">{rows}</div>
<p class="muted" style="margin:4px 0 0 16px">The last 100 matching events from the actions audit log.</p>"#,
            head = group_head(group, None),
            filter_bar = filter_bar,
            rows = rows,
        ),
        true,
    )
    .into_response()
}

// --- danger zone: delete all content / delete the whole site ---

/// Tables wiped by "delete all content" (Ghost's danger zone keeps users and
/// settings) — everything post/tag-shaped plus their join and revision tables.
pub(crate) const CONTENT_TABLES: &[&str] = &[
    "posts", "posts_meta", "posts_tags", "posts_authors", "posts_products",
    "post_revisions", "mobiledoc_revisions", "tags", "snippets", "emails",
    "email_batches", "email_recipients", "email_recipient_failures",
];

/// Tables wiped by "delete site" on top of the content set — identity, members
/// and commerce data. Settings, roles and permissions survive so first-boot
/// seeding can run again.
pub(crate) const SITE_TABLES: &[&str] = &[
    "users", "sessions", "actions", "invites", "roles_users",
    "members", "members_products", "members_newsletters", "members_labels",
    "members_created_events", "members_stripe_customers", "subscriptions",
    "products", "products_benefits", "benefits", "offers",
    "stripe_products", "stripe_prices",
    "newsletters", "recommendations", "recommendation_click_events",
    "recommendation_subscribe_events", "labels",
    "comments", "comment_likes", "comment_reports",
    "integrations", "api_keys", "webhooks", "custom_theme_settings",
    "brute", "tokens", "milestones",
    // Responses are visitors' personal data, so deleting the site deletes
    // them; children first, so it holds whether or not the engine cascades.
    "form_submissions", "forms",
];

pub(crate) async fn wipe_tables(db: &ghost_db::Db, tables: &[&str]) -> usize {
    let mut wiped = 0;
    for table in tables {
        if sqlx::query(&format!("DELETE FROM {table}")).execute(db).await.is_ok() {
            wiped += 1;
        }
    }
    wiped
}

pub(crate) async fn danger_page(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let group = SETTINGS_GROUPS.iter().find(|g| g.slug == "danger").unwrap();
    page_for_role(actor_role, 
        "Danger zone",
        "settings",
        Some(&user),
        &format!(
            r#"{head}{banner}
<div class="section">Danger zone</div>
<div class="card rows rise" style="--i:1">
<div class="row"><span class="ic tint-red">{posts}</span>
<div class="body"><span class="title">Delete all content</span><span class="detail">Permanently remove every post, page, tag and email draft. Users and settings are kept.</span></div>
<div class="trailing">
<form method="post" action="/ghost/settings/danger/content/" data-confirm="Delete all content|Every post, page and tag will be permanently deleted. This cannot be undone.">
<button class="btn text danger" type="submit">Delete…</button></form></div></div>
<div class="row"><span class="ic tint-red">{lock}</span>
<div class="body"><span class="title">Reset all authentication</span><span class="detail">Sign every staff member and member out everywhere, and invalidate every outstanding magic link. Nobody loses their account.</span></div>
<div class="trailing">
<form method="post" action="/ghost/settings/danger/auth/" data-confirm="Reset all authentication|Everyone — staff and members — will be signed out on every device, and any magic links already emailed will stop working. You will be signed out too.">
<button class="btn text danger" type="submit">Reset…</button></form></div></div>
<div class="row"><span class="ic tint-red">{gift}</span>
<div class="body"><span class="title">Reset all gift links</span><span class="detail">Invalidate every gift subscription link that has been shared but not yet redeemed.</span></div>
<div class="trailing">
<form method="post" action="/ghost/settings/danger/gifts/" data-confirm="Reset all gift links|Every unredeemed gift link stops working. Gifts already redeemed are unaffected.">
<button class="btn text danger" type="submit">Reset…</button></form></div></div>
<div class="row"><span class="ic tint-red">{ghost}</span>
<div class="body"><span class="title">Delete site</span><span class="detail">Wipe everything — content, users, members and integrations — and restart at the setup screen. Settings return to their defaults.</span></div>
<div class="trailing">
<form method="post" action="/ghost/settings/danger/site/" data-confirm="Delete this site|ALL content, users and integrations will be permanently deleted and you will be signed out. This cannot be undone.">
<button class="btn text danger" type="submit">Delete…</button></form></div></div>
</div>
<p class="muted" style="margin:14px 0 0 16px">Ghost version {version} · {env} · SQLite · <a href="/ghost/settings/about/">About this site</a></p>"#,
            head = group_head(group, None),
            banner = flash_banner(&q),
            posts = svg("delete"),
            lock = svg("lock"),
            gift = svg("mail"),
            ghost = svg("ghost"),
            version = GHOST_VERSION,
            env = esc(&state.config.env),
        ),
        true,
    )
    .into_response()
}

pub(crate) async fn danger_delete_content(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let (user, _actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    let _ = wipe_tables(&state.db, CONTENT_TABLES).await;
    let _ = repos::actions::record(&state.db, "db", "content", &user.id, "deleted").await;
    settings_flash("danger", "ok", "All content deleted.")
}

pub(crate) async fn danger_delete_site(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let _ = wipe_tables(&state.db, CONTENT_TABLES).await;
    let _ = wipe_tables(&state.db, SITE_TABLES).await;
    state.refresh_setup_flag().await;
    // the session row is gone too — straight back to setup
    Redirect::to("/ghost/setup/").into_response()
}

/// Sign everyone out, everywhere.
///
/// Ghost's "Reset all authentication": staff sessions, member sessions and
/// every unredeemed magic-link token go at once. Nobody loses their account —
/// they sign in again. The caller is signed out too, which is the point: an
/// operator reaching for this has lost track of who holds a session, and an
/// exception for themselves would leave the one session they cannot vouch for.
pub(crate) async fn danger_reset_auth(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    // Shared with `POST /authentication/reset`, so the button and the API
    // revoke the same things. This also rotates every API key secret and locks
    // every staff account, which signing sessions out alone does not: a
    // stolen admin key or a known password would otherwise still work.
    let actor = current_user(&state, &headers).await.map(|u| u.id);
    if let Err(e) = ghost_api::reset_auth::reset_authentication(&state.db, actor.as_deref()).await {
        return settings_flash("danger", "error", &format!("Could not reset authentication: {}", e.message()));
    }

    // No session of our own survives, so there is nowhere to flash to.
    Redirect::to("/ghost/signin/?reset=1").into_response()
}

/// Invalidate every unredeemed gift link.
///
/// A gift link is a token like a magic link; redeemed gifts are subscriptions
/// and are untouched.
pub(crate) async fn danger_reset_gifts(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = require_role(&state, &headers, Role::Administrator).await {
        return r;
    }
    let removed = sqlx::query("DELETE FROM tokens WHERE data LIKE '%\"gift\"%'")
        .execute(&state.db)
        .await
        .map(|r| r.rows_affected())
        .unwrap_or(0);
    settings_flash(
        "danger",
        "ok",
        &match removed {
            0 => "No unredeemed gift links to reset.".to_string(),
            1 => "1 gift link reset.".to_string(),
            n => format!("{n} gift links reset."),
        },
    )
}

/// Ghost's About dialog: what is running, and where.
pub(crate) async fn about_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let (user, actor_role) = match require_role(&state, &headers, Role::Administrator).await {
        Ok(a) => (a.user, a.role),
        Err(r) => return r,
    };
    // What to show for the database depends on the engine, and on MySQL it
    // must be the *redacted* connection — this page is visible to any
    // administrator of the site, and on a hosted site the password belongs to
    // the platform rather than to them.
    let db_where = match state.config.db_target() {
        Ok(target) => match target.sqlite_path() {
            Some(path) => path.display().to_string(),
            None => target.redacted(),
        },
        // A config this build cannot open would have stopped the server long
        // before here, so this is only for completeness.
        Err(e) => e.to_string(),
    };
    let mail = match state.settings.get_str("mail_transport").as_deref() {
        Some("smtp") => format!(
            "SMTP · {}",
            state.settings.get_str("mail_smtp_host").unwrap_or_else(|| "not configured".into())
        ),
        _ => "Direct".to_string(),
    };
    let row = |label: &str, value: &str| {
        format!(
            r#"<div class="row"><div class="body"><span class="title">{label}</span><span class="detail">{value}</span></div></div>"#,
            label = esc(label),
            value = esc(value),
        )
    };

    page_for_role(actor_role,
        "About",
        "settings",
        Some(&user),
        &format!(
            r#"<header class="page-head rise"><a class="icon-btn" style="transform:rotate(180deg)" href="/ghost/settings/" title="Back to settings">{back}</a>
<div><h1>About</h1><p class="sub">What is running on this site</p></div></header>
<div class="card rows rise" style="--i:1">
{version_row}{port_row}{env_row}{db_row}{mail_row}{theme_row}
</div>
<p class="muted" style="margin:14px 0 0 16px">Vuuut is an independent reimplementation of Ghost in Rust. It tracks the Ghost
release above for its admin, Content API and theme behaviour; it is not affiliated with the Ghost Foundation.</p>"#,
            back = svg("chevron"),
            version_row = row("Ghost version", GHOST_VERSION),
            port_row = row("Vuuut", env!("CARGO_PKG_VERSION")),
            env_row = row("Environment", &state.config.env),
            db_row = row("Database", &format!("{} · {}", state.config.database.client, db_where)),
            mail_row = row("Mail", &mail),
            theme_row = row(
                "Active theme",
                &state.settings.get_str("active_theme").unwrap_or_else(|| "casper".into()),
            ),
        ),
        true,
    )
    .into_response()
}

// -------------------------------------------------------------------------
// Two settings pages whose forms are built by a script rather than server-rendered.
// -------------------------------------------------------------------------

pub(crate) async fn machine_payments_page(State(state):State<AppState>,headers:HeaderMap)->Response{
    let actor=match require_role(&state,&headers,Role::Administrator).await{Ok(actor)=>actor,Err(response)=>return response};
    let group=SETTINGS_GROUPS.iter().find(|g|g.slug=="machine-payments").unwrap();
    page_for_role(actor.role,"Machine payments","settings",Some(&actor.user),&format!(
        r#"{head}<div id="machine-payments"><div class="card padded"><p class="muted" role="status" style="margin:0">Loading payment settings…</p></div></div><script src="{assets}/machine-payments.js" defer></script>"#,
        // No header Save button: the form the JS builds carries its own, and
        // saving has to run through the JSON API rather than a form POST.
        // The host is a bare container because the page builds its own
        // section cards — a card around all of them nests one inside another.
        head=group_head(group,None),
        assets=ASSETS,
    ),true).into_response()
}

pub(crate) async fn checkout_design_page(State(state):State<AppState>,headers:HeaderMap)->Response {
    let actor=match require_role(&state,&headers,Role::Administrator).await {Ok(actor)=>actor,Err(response)=>return response};
    page_for_role(actor.role,"Checkout design","settings",Some(&actor.user),&format!(r#"<header class="page-head"><div><h1>Checkout design</h1><p class="sub">Style the payment page for paid memberships</p></div></header><div id="checkout-design"><div class="card padded"><p role="status">Loading checkout design…</p></div></div><script src="{assets}/checkout-design.js" defer></script>"#, assets = ASSETS),true).into_response()
}
