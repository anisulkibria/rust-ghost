//! Vuuut boot sequence.
//!
//! config → db + migrate + seed → settings cache → routes.yaml → redirects
//! → theme activation → build app → listen.
mod admin_address;
mod cli;
mod serve;

use vuuut_api::AppState;
use vuuut_db::repos;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    // The commands that only read or write files print plainly and exit;
    // installing a log subscriber first would dress their output up as server
    // logs.
    let command = cli::parse();
    match &command {
        cli::Command::Help => return cli::print_help(),
        cli::Command::Version => return cli::print_version(),
        cli::Command::Setup(args) => {
            if let Err(e) = cli::setup(args) {
                eprintln!("\nsetup failed: {e}\n");
                std::process::exit(1);
            }
            return;
        }
        cli::Command::Systemd(args) => {
            if let Err(e) = cli::systemd(args) {
                eprintln!("{e}");
                std::process::exit(1);
            }
            return;
        }
        cli::Command::Doctor | cli::Command::SeedDemo | cli::Command::SeedDirectory | cli::Command::ImportContent(_) | cli::Command::Start => {}
    }

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    if matches!(command, cli::Command::Doctor) {
        println!("\nChecking this install\n");
        std::process::exit(match cli::doctor().await {
            true => 0,
            false => 1,
        });
    }

    let started = std::time::Instant::now();

    // 1. config
    let config = Arc::new(vuuut_core::Config::load(None).expect("config"));
    tracing::info!("Vuuut booting (env: {})", config.env);

    // The base path from `config.url` decides both where the app mounts and
    // how every URL it writes is prefixed. Publish it before anything that
    // renders or serializes can run.
    let subdir = vuuut_core::url::init(&config.subdir()).expect("base path");
    if !subdir.is_empty() {
        tracing::info!("installed under base path {subdir} ({})", config.canonical_url());
    }

    // Resolve how to listen before doing any work: a contradiction between
    // `url` and `ssl` should fail in the first second with the fix in the
    // message, not after migrating a database.
    let plan = match config.serve_plan() {
        Ok(plan) => plan,
        Err(e) => {
            tracing::error!("{}", e.message());
            std::process::exit(1);
        }
    };

    // ensure content dirs
    for dir in [
        config.content_path(),
        config.themes_dir(),
        config.data_dir(),
        config.images_dir(),
        config.files_dir(),
        config.media_dir(),
        config.content_path().join("public"),
        config.content_path().join("logs"),
    ] {
        let _ = tokio::fs::create_dir_all(&dir).await;
    }

    // 2. database + migrations
    //
    // SQLite or MySQL, decided by the `database` block; see
    // `vuuut_core::Config::db_target`.
    let target = config.db_target().expect("database configuration");
    let db = vuuut_db::pool::open(&target).await.expect("database");
    // Redacted: on MySQL this url carries the password, and this line goes to
    // a log the site's own administrators can read.
    tracing::info!("database ready: {}", target.redacted());

    // 3. seed on first boot
    if vuuut_db::seed::seed_if_empty(&db).await.expect("seed") {
        tracing::info!("seeded initial data (roles, settings, fixture posts)");
    }
    vuuut_db::seed::populate_missing_settings(&db).await.expect("settings defaults");
    vuuut_db::seed::ensure_builtin_integrations(&db).await.expect("builtin integrations");
    vuuut_db::seed::ensure_default_automations(&db).await.expect("default automations");

    // Brute-force records outlive their own window; clearing the expired ones
    // at boot keeps the table from growing for the lifetime of the install.
    vuuut_api::brute::sweep(&db).await;

    // Content written before Ghost 5 is stored as Mobiledoc, which nothing
    // renders any more. Converting it once here means a legacy post nobody
    // edits is still readable and editable.
    match vuuut_render::mobiledoc::backfill_legacy_posts(&db).await {
        Ok(0) => {}
        Ok(converted) => tracing::info!("converted {converted} legacy Mobiledoc post(s) to Lexical"),
        Err(e) => tracing::warn!("Mobiledoc conversion skipped: {e}"),
    }

    // `vuuut seed-demo` fills the site with demo content and exits, rather
    // than starting the server
    if matches!(command, cli::Command::SeedDemo) {
        match vuuut_db::demo::seed_demo(&db).await {
            Ok(s) => {
                tracing::info!(
                    "demo content seeded: {} posts ({} featured, {} replaced) across {} tags",
                    s.posts,
                    s.featured,
                    s.replaced,
                    s.tags
                );
            }
            Err(e) => {
                tracing::error!("demo seeding failed: {}", e.message());
                std::process::exit(1);
            }
        }
        return;
    }

    // `vuuut import-content demo.json [--with-settings]`: load a Ghost export
    // (a theme's demo content) through Universal import's own path, then
    // exit. Site settings in the file (menus, title) are skipped unless
    // --with-settings is given, so a demo never rewrites a site's menus.
    if let cli::Command::ImportContent(args) = &command {
        let Some(path) = args.get("file") else {
            eprintln!("usage: vuuut import-content <export.json> [--with-settings]");
            std::process::exit(2);
        };
        let raw = std::fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("cannot read {path}: {e}");
            std::process::exit(1)
        });
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap_or_else(|e| {
            eprintln!("{path} is not JSON: {e}");
            std::process::exit(1)
        });
        let mut data = parsed.pointer("/db/0/data").cloned().unwrap_or(parsed);
        if !args.flag("with-settings") {
            if let Some(m) = data.as_object_mut() {
                m.remove("settings");
            }
        }
        let owner = match vuuut_db::repos::users::find_owner(&db).await {
            Ok(Some(u)) => u.id,
            _ => {
                eprintln!("this site has no owner yet; finish setup first");
                std::process::exit(1);
            }
        };
        let posts = data["posts"].as_array().map(|p| p.len()).unwrap_or(0);
        let prepared = vuuut_api::publishing::prepare_content_import(&data);
        match vuuut_db::transfer::import(&db, &prepared, &owner).await {
            Ok(()) => tracing::info!("imported {posts} posts and pages from {path}; restart the server to see them"),
            Err(e) => {
                tracing::error!("import failed: {}", e.message());
                std::process::exit(1);
            }
        }
        return;
    }

    // `vuuut seed-directory`: the directory demo as one dummy batch (Tools →
    // Dummy content removes it), then exit.
    if matches!(command, cli::Command::SeedDirectory) {
        let settings = vuuut_db::SettingsCache::load(&db).await.expect("settings");
        match vuuut_admin_ui::dummies::seed_directory_demo(&db, &settings).await {
            Ok(made) => tracing::info!("directory demo seeded: {made}"),
            Err(e) => {
                tracing::error!("directory seeding failed: {}", e.message());
                std::process::exit(1);
            }
        }
        return;
    }

    // 4. settings cache
    let settings = Arc::new(vuuut_db::SettingsCache::load(&db).await.expect("settings").with_mail_config(&config.mail).with_managed_mail(&config.host_settings));

    // Apply the Security module's session policy to what is already in the
    // table: expire anything past its maximum age or idle limit, and drop
    // sessions whose user no longer exists. This has to come after the
    // settings cache, because the policy is two of its rows.
    //
    // It runs at boot rather than on a timer deliberately. The policy is
    // enforced on every request by
    // `vuuut_db::repos::sessions::get_user_within`, which refuses an expired
    // session *and* deletes it — so the correctness of the limit does not
    // depend on this call happening at all. What this does is clear rows
    // nobody will ever ask about again, which is housekeeping and does not
    // need to be timely. A periodic task would be a second, slower answer to
    // a question the guard already answers immediately.
    {
        let policy = vuuut_core::security::session_max_age_secs(&serde_json::json!({
            "security_session_days": settings.get("security_session_days"),
        }));
        let idle = vuuut_core::security::session_idle_secs(&serde_json::json!({
            "security_session_idle_minutes": settings.get("security_session_idle_minutes"),
        }));
        vuuut_db::repos::security::sweep(&db, policy, idle).await;
    }

    // Firewall rules whose expiry has passed. Reads already filter them, so
    // this only keeps the table from growing with rules nobody will ever see
    // again.
    match vuuut_db::repos::firewall::sweep_expired(&db).await {
        Ok(0) => {}
        Ok(n) => tracing::info!("firewall: removed {n} expired rule(s)"),
        Err(e) => tracing::warn!("firewall: could not sweep expired rules: {}", e.message()),
    }

    // 5. routes.yaml
    let routes_path = config.content_path().join("routes.yaml");
    let routes_yaml = match tokio::fs::read_to_string(&routes_path).await {
        Ok(y) => y,
        Err(_) => {
            tokio::fs::write(&routes_path, vuuut_routing::DEFAULT_ROUTES_YAML).await.ok();
            vuuut_routing::DEFAULT_ROUTES_YAML.to_string()
        }
    };
    let route_settings = vuuut_routing::routes_yaml::parse_routes_yaml(&routes_yaml)
        .unwrap_or_else(|e| {
            tracing::error!("routes.yaml invalid ({e}); using defaults");
            vuuut_routing::routes_yaml::parse_routes_yaml(vuuut_routing::DEFAULT_ROUTES_YAML).unwrap()
        });
    let router_tree = Arc::new(vuuut_routing::RouterTree::build(&route_settings).expect("routes"));

    // 6. redirects.yaml
    let redirects = vuuut_routing::redirects::load_from(&config.content_path().join("redirects.yaml"))
        .await
        .unwrap_or_default();

    // 7. theme activation — the configured theme first, then any other
    // installed one, so a theme that fails to compile cannot leave the site
    // (and the admin UI you'd fix it from) unbootable
    let active_name = settings.get_str("active_theme").unwrap_or_else(|| "source".into());
    let themes = vuuut_render::list_themes(&config.themes_dir());
    let mut candidates: Vec<_> = themes.iter().filter(|(name, _)| *name == active_name).collect();
    candidates.extend(themes.iter().filter(|(name, _)| *name != active_name));

    if candidates.is_empty() {
        tracing::error!("no themes found in {}", config.themes_dir().display());
        std::process::exit(1);
    }
    let site_origin = settings
        .get_str("site_url")
        .unwrap_or_else(|| config.url.clone())
        .trim_end_matches('/')
        .to_string();

    let mut activated = None;
    for (name, dir) in candidates {
        match vuuut_api::themes::build_theme_state(db.clone(), router_tree.clone(), site_origin.clone(), dir, &settings).await {
            Ok(theme) => {
                activated = Some((name.clone(), theme));
                break;
            }
            Err(e) => tracing::error!("theme '{name}' failed to load: {}", e.message()),
        }
    }
    let Some((theme_name, theme)) = activated else {
        tracing::error!("no usable theme in {}", config.themes_dir().display());
        std::process::exit(1);
    };

    // Built before the state literal, which takes ownership of `db`.
    let firewall_cache = match vuuut_db::repos::firewall::RuleCache::load(&db).await {
        Ok(cache) => cache,
        Err(e) => {
            tracing::error!("firewall: could not load the rules, starting with none: {}", e.message());
            vuuut_db::repos::firewall::RuleCache::empty()
        }
    };
    let state = AppState {
        db,
        config: config.clone(),
        settings: settings.clone(),
        theme: Arc::new(RwLock::new(Arc::new(theme))),
        router: Arc::new(RwLock::new(router_tree)),
        redirects: Arc::new(RwLock::new(Arc::new(redirects))),
        setup_done: Arc::new(AtomicBool::new(false)),
        frontend_key: Arc::new(RwLock::new(None)),
        limits: Arc::new(vuuut_db::limits::Limits::load(&config.host_settings)),
        // Loaded here rather than lazily on the first request: the firewall
        // is consulted before anything else, and a first request that had to
        // wait on a query to find out whether it was allowed would be the one
        // request most likely to be arriving in a flood. A load that fails
        // gives an **empty** firewall, which lets everybody through — the
        // same choice `check` makes for an address it cannot parse, and for
        // the same reason: a database hiccup must not take the site off the
        // internet.
        firewall: firewall_cache,

    };
    state.refresh_setup_flag().await;
    vuuut_admin_ui::builder::refresh_elementor_layouts(&state).await;
    // the configured theme may be gone — record whichever one we fell back to
    if theme_name != active_name {
        let _ = settings.set(&state.db, "active_theme", &serde_json::json!(theme_name)).await;
    }
    tracing::info!("theme '{}' activated", theme_name);

    // log the content API key for immediate API testing
    if let Ok(Some(key)) = repos::integrations::internal_content_key(&state.db).await {
        tracing::info!("Content API key (ghost-core-content): {key}");
    }

    // 8. start the publisher for scheduled posts, then build the app mounted
    // under the base path. The publisher runs for the life of the process:
    // a post scheduled for Friday has nobody making a request at the moment
    // it is due, so nothing in the request path can publish it.
    vuuut_admin_ui::schedule::spawn(state.clone());
    // The automations engine. Like the publisher above it runs for the life
    // of the process: a member waiting three days for the next email in a
    // chain has nobody making a request when it comes due.
    vuuut_api::automations_engine::spawn(state.clone());
    vuuut_api::members::spawn_gift_delivery(state.clone());
    vuuut_api::email_analytics::spawn(state.clone());
    // Ships operational rows to the analytics warehouse, if one is
    // configured. A self-hosted site has none and this does nothing.
    vuuut_api::tinybird_sync::spawn(state.clone());
    vuuut_api::click::spawn(state.clone());
    vuuut_api::explore::spawn(state.clone());
    // Automatic backups, on the schedule set in Settings → Backups.
    vuuut_api::backups::spawn(state.clone());
    vuuut_api::federation::spawn(state.clone());
    let admin_url = state.admin_url();
    let app = mount(build_app(state.clone()), subdir, state);

    // 9. listen
    tracing::info!(
        "Vuuut ready in {:.2}s — {} (admin at {}/)",
        started.elapsed().as_secs_f64(),
        config.canonical_url(),
        admin_url,
    );
    if let Err(e) = serve::run(&plan, app, &config.canonical_url()).await {
        tracing::error!("server stopped: {e}");
        std::process::exit(1);
    }
}

/// Mount the whole app under `subdir` by stripping the base path off incoming
/// requests, so every route inside the app stays declared at the root.
///
/// The app ends up reachable at the base path *and* at the server root. That
/// is deliberate: a reverse proxy fronting a subdirectory install may either
/// forward the prefix through —
///
/// ```nginx
/// location /blog/ { proxy_pass http://127.0.0.1:1971; }       # keeps /blog/
/// ```
///
/// — or strip it before forwarding:
///
/// ```nginx
/// location /blog/ { proxy_pass http://127.0.0.1:1971/; }      # strips /blog/
/// ```
///
/// Both are things people write, and which one you get turns on a single
/// trailing slash. Accepting either means the install works without the
/// operator having to know which convention their proxy follows — and the URLs
/// the app *writes* always carry the base path, so links stay correct either
/// way.
///
/// Rewriting the request *before* routing is what makes this work: the strip
/// has to happen ahead of route matching, so it is a layer wrapped around the
/// router rather than `Router::layer` (which only wraps the matched handler,
/// by which point the path has already been matched against the unstripped
/// URI). Nesting would not do either — the site router owns a `/{*path}`
/// catch-all, and a nested router alongside that catch-all leaves the base
/// path's own routes shadowed.
///
/// With no base path configured the layer passes every request through
/// untouched.
fn mount(
    app: axum::Router,
    subdir: &'static str,
    state: AppState,
) -> impl tower::Service<
    axum::extract::Request,
    Response = axum::response::Response,
    Error = std::convert::Infallible,
    Future = impl Send,
> + Clone
+ Send
+ 'static {
    use axum::extract::Request;
    use axum::http::uri::{PathAndQuery, Uri};
    use axum::response::IntoResponse;
    use tower::Layer;

    axum::middleware::from_fn_with_state(state, move |axum::extract::State(state): axum::extract::State<AppState>, mut req: Request, next: axum::middleware::Next| async move {
        if !subdir.is_empty() {
            let path = req.uri().path();

            // `/blog` → `/blog/`: without the trailing slash a browser
            // resolves the page's relative links against the parent directory
            if path == subdir {
                return axum::response::Redirect::permanent(&format!("{subdir}/")).into_response();
            }

            if let Some(rest) = path.strip_prefix(subdir).filter(|r| r.starts_with('/')) {
                let mut parts = req.uri().clone().into_parts();
                let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
                if let Ok(pq) = format!("{rest}{query}").parse::<PathAndQuery>() {
                    parts.path_and_query = Some(pq);
                    if let Ok(uri) = Uri::from_parts(parts) {
                        *req.uri_mut() = uri;
                    }
                }
            }
        }
        admin_address::route(&state, req, next).await
    })
    .layer(app)
}

fn build_app(state: AppState) -> axum::Router {
    

    let site = vuuut_front::site_router(state.clone());

    // /ghost/api/content/* (+ legacy /vN/ prefixes tolerated like Ghost)
    let api = axum::Router::new()
        .nest("/content", vuuut_api::content::content_router(state.clone()))
        .nest("/admin", vuuut_api::admin::admin_router()
            .merge(vuuut_api::admin_comments::admin_comments_router())
            .merge(vuuut_api::automations::automations_router())
            // Retrying an email means rendering and sending it again, which
            // lives in the admin crate; the rest of the emails family is in
            // `vuuut-api`. Merged here so the whole family shares a prefix.
            .merge(vuuut_admin_ui::email_retry_router())
            // Import, export, themes and the settings files: the operations
            // the admin already performs, as the REST a client can reach.
            .merge(vuuut_admin_ui::rest::router())
            .merge(axum::Router::new()
                .nest("/images", vuuut_api::images::images_router())
                .nest("/media", vuuut_api::images::media_router())
                .nest("/files", vuuut_api::images::files_router())
                // the editor's bookmark and embed cards resolve URLs here
                .merge(vuuut_api::oembed::oembed_router())))
        // An integration key that names no staff user resolves to the owner,
        // so the role hierarchy cannot distinguish it from one. Ghost gives
        // integrations a permission matrix of their own; this applies it.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            vuuut_api::integration_permissions::enforce,
        ))
        .with_state(state.clone());

    // The invisible iframe the comments widget talks to when a moderator is
    // signed in. It lives under /ghost because that is where the admin
    // session cookie is scoped — the whole point is that it can reach the
    // Admin API with credentials, which a script on the site origin cannot.
    let auth_frame = axum::Router::new()
        .route("/auth-frame/", axum::routing::get(vuuut_api::admin_comments::auth_frame))
        .route("/auth-frame", axum::routing::get(vuuut_api::admin_comments::auth_frame))
        .route(
            "/auth-frame/admin-auth.min.js",
            axum::routing::get(vuuut_api::admin_comments::auth_frame_script),
        )
        .with_state(state.clone());

    // Older Ghost SDKs use versioned URLs instead of Accept-Version.
    let mut compatible_api = api.clone();
    for prefix in ["/v2", "/v3", "/v4", "/v5", "/v6"] {
        compatible_api = compatible_api.nest(prefix, api.clone());
    }
    let backend = axum::Router::new().nest("/api", compatible_api).merge(auth_frame);

    // built-in admin UI at /ghost
    let admin_ui = vuuut_admin_ui::admin_ui_router(state.clone()).with_state(state.clone());

    // /members/* — Portal's backend. Merged (not nested) at the site root,
    // since Portal builds these URLs from the site origin and carries the
    // /members prefix itself.
    // Endpoints a stranger or another site has to be able to reach: the
    // webmention receiver, the recommendations file other Ghost sites read,
    // the public half of the signing key, and a member's entitlement token.
    let open_endpoints = vuuut_api::mentions::public_router()
        .merge(vuuut_api::identity::public_router())
        .merge(vuuut_api::automation_tracking::public_router())
        .with_state(state.clone());

    let members = vuuut_api::members::members_router()
        // `/members/api/comments/*` — where the CDN-hosted comments widget
        // is hardcoded to look, so it is merged at the site root alongside
        // the rest of the members API rather than nested.
        .merge(vuuut_api::comments::comments_router())
        .merge(vuuut_api::payments::router())
        .with_state(state.clone());

    // `nest` binds the admin UI's own `/` route at `/ghost` but not at
    // `/ghost/`, so the canonical admin URL would otherwise fall through to
    // the site's `/{*path}` catch-all and bounce back to itself.
    let admin_root = vuuut_admin_ui::admin_root_router().with_state(state.clone());

    site.nest("/ghost", backend.merge(admin_ui))
        .merge(admin_root)
        .merge(members)
        .merge(open_endpoints)
}

