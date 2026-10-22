//! Ghost-rust boot sequence.
//!
//! config → db + migrate + seed → settings cache → routes.yaml → redirects
//! → theme activation → build app → listen.
mod cli;
mod serve;

use ghost_api::AppState;
use ghost_db::repos;
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
        cli::Command::Doctor | cli::Command::SeedDemo | cli::Command::Start => {}
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
    let config = Arc::new(ghost_core::Config::load(None).expect("config"));
    tracing::info!("Ghost-rust booting (env: {})", config.env);

    // The base path from `config.url` decides both where the app mounts and
    // how every URL it writes is prefixed. Publish it before anything that
    // renders or serializes can run.
    let subdir = ghost_core::url::init(&config.subdir()).expect("base path");
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
    let db = ghost_db::connect(&config.db_path()).await.expect("database");
    tracing::info!("database ready at {}", config.db_path().display());

    // 3. seed on first boot
    if ghost_db::seed::seed_if_empty(&db).await.expect("seed") {
        tracing::info!("seeded initial data (roles, settings, fixture posts)");
    }
    ghost_db::seed::populate_missing_settings(&db).await.expect("settings defaults");
    ghost_db::seed::ensure_builtin_integrations(&db).await.expect("builtin integrations");
    ghost_db::seed::ensure_default_automations(&db).await.expect("default automations");

    // `ghost seed-demo` fills the site with demo content and exits, rather
    // than starting the server
    if matches!(command, cli::Command::SeedDemo) {
        match ghost_db::demo::seed_demo(&db).await {
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

    // 4. settings cache
    let settings = Arc::new(ghost_db::SettingsCache::load(&db).await.expect("settings"));

    // 5. routes.yaml
    let routes_path = config.content_path().join("routes.yaml");
    let routes_yaml = match tokio::fs::read_to_string(&routes_path).await {
        Ok(y) => y,
        Err(_) => {
            tokio::fs::write(&routes_path, ghost_routing::DEFAULT_ROUTES_YAML).await.ok();
            ghost_routing::DEFAULT_ROUTES_YAML.to_string()
        }
    };
    let route_settings = ghost_routing::routes_yaml::parse_routes_yaml(&routes_yaml)
        .unwrap_or_else(|e| {
            tracing::error!("routes.yaml invalid ({e}); using defaults");
            ghost_routing::routes_yaml::parse_routes_yaml(ghost_routing::DEFAULT_ROUTES_YAML).unwrap()
        });
    let router_tree = Arc::new(ghost_routing::RouterTree::build(&route_settings).expect("routes"));

    // 6. redirects.yaml
    let redirects = ghost_routing::redirects::load_from(&config.content_path().join("redirects.yaml"))
        .await
        .unwrap_or_default();

    // 7. theme activation — the configured theme first, then any other
    // installed one, so a theme that fails to compile cannot leave the site
    // (and the admin UI you'd fix it from) unbootable
    let active_name = settings.get_str("active_theme").unwrap_or_else(|| "source".into());
    let themes = ghost_render::list_themes(&config.themes_dir());
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

    let locale = settings.get_str("locale").unwrap_or_else(|| "en".into());

    let mut activated = None;
    for (name, dir) in candidates {
        match ghost_api::themes::build_theme_state(db.clone(), router_tree.clone(), site_origin.clone(), dir, &locale).await {
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

    let state = AppState {
        db,
        config: config.clone(),
        settings: settings.clone(),
        theme: Arc::new(RwLock::new(Arc::new(theme))),
        router: Arc::new(RwLock::new(router_tree)),
        redirects: Arc::new(RwLock::new(Arc::new(redirects))),
        setup_done: Arc::new(AtomicBool::new(false)),
        frontend_key: Arc::new(RwLock::new(None)),
    };
    state.refresh_setup_flag().await;
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
    ghost_admin_ui::schedule::spawn(state.clone());
    // The automations engine. Like the publisher above it runs for the life
    // of the process: a member waiting three days for the next email in a
    // chain has nobody making a request when it comes due.
    ghost_api::automations_engine::spawn(state.clone());
    let app = mount(build_app(state), subdir);

    // 9. listen
    tracing::info!(
        "Ghost-rust ready in {:.2}s — {} (admin at {}/ghost/)",
        started.elapsed().as_secs_f64(),
        config.canonical_url(),
        config.canonical_url()
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
/// location /blog/ { proxy_pass http://127.0.0.1:2368; }       # keeps /blog/
/// ```
///
/// — or strip it before forwarding:
///
/// ```nginx
/// location /blog/ { proxy_pass http://127.0.0.1:2368/; }      # strips /blog/
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

    axum::middleware::from_fn(move |mut req: Request, next: axum::middleware::Next| async move {
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
        next.run(req).await
    })
    .layer(app)
}

fn build_app(state: AppState) -> axum::Router {
    

    let site = ghost_front::site_router(state.clone());

    // /ghost/api/content/* (+ legacy /vN/ prefixes tolerated like Ghost)
    let api = axum::Router::new()
        .nest("/content", ghost_api::content::content_router())
        .nest("/admin", ghost_api::admin::admin_router()
            .merge(ghost_api::admin_comments::admin_comments_router())
            .merge(ghost_api::automations::automations_router())
            .merge(axum::Router::new()
                .nest("/images", ghost_api::images::images_router())
                .nest("/media", ghost_api::images::media_router())
                .nest("/files", ghost_api::images::files_router())
                // the editor's bookmark and embed cards resolve URLs here
                .merge(ghost_api::oembed::oembed_router())))
        .with_state(state.clone());

    // The invisible iframe the comments widget talks to when a moderator is
    // signed in. It lives under /ghost because that is where the admin
    // session cookie is scoped — the whole point is that it can reach the
    // Admin API with credentials, which a script on the site origin cannot.
    let auth_frame = axum::Router::new()
        .route("/auth-frame/", axum::routing::get(ghost_api::admin_comments::auth_frame))
        .route("/auth-frame", axum::routing::get(ghost_api::admin_comments::auth_frame))
        .route(
            "/auth-frame/admin-auth.min.js",
            axum::routing::get(ghost_api::admin_comments::auth_frame_script),
        )
        .with_state(state.clone());

    let backend = axum::Router::new().nest("/api", api).merge(auth_frame);

    // built-in admin UI at /ghost
    let admin_ui = ghost_admin_ui::admin_ui_router().with_state(state.clone());

    // /members/* — Portal's backend. Merged (not nested) at the site root,
    // since Portal builds these URLs from the site origin and carries the
    // /members prefix itself.
    let members = ghost_api::members::members_router()
        // `/members/api/comments/*` — where the CDN-hosted comments widget
        // is hardcoded to look, so it is merged at the site root alongside
        // the rest of the members API rather than nested.
        .merge(ghost_api::comments::comments_router())
        .with_state(state.clone());

    // `nest` binds the admin UI's own `/` route at `/ghost` but not at
    // `/ghost/`, so the canonical admin URL would otherwise fall through to
    // the site's `/{*path}` catch-all and bounce back to itself.
    let admin_root = ghost_admin_ui::admin_root_router().with_state(state.clone());

    site.nest("/ghost", backend.merge(admin_ui))
        .merge(admin_root)
        .merge(members)
}

