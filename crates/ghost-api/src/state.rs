//! Shared application state.
use ghost_core::Config;
use ghost_db::SettingsCache;
use ghost_render::ThemeState;
use ghost_routing::redirects::Redirects;
use ghost_routing::RouterTree;
use sqlx::SqlitePool;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub config: Arc<Config>,
    pub settings: Arc<SettingsCache>,
    pub theme: Arc<RwLock<Arc<ThemeState>>>,
    pub router: Arc<RwLock<Arc<RouterTree>>>,
    pub redirects: Arc<RwLock<Arc<Redirects>>>,
    pub setup_done: Arc<AtomicBool>,
    /// The internal frontend Content API key, memoised. Every rendered page
    /// stamps it into `ghost_head`, so it must not cost a query per request —
    /// but it does not exist until setup has run, hence the `Option` and the
    /// lazy fill rather than a value resolved at boot.
    pub frontend_key: Arc<RwLock<Option<String>>>,
}

impl AppState {
    pub fn site_url(&self) -> String {
        self.settings
            .get_str("site_url")
            .unwrap_or_else(|| self.config.url.clone())
            .trim_end_matches('/')
            .to_string()
    }

    pub fn admin_url(&self) -> String {
        format!("{}/ghost", self.site_url())
    }

    /// The frontend Content API key, reading through to the DB once.
    pub async fn frontend_key(&self) -> Option<String> {
        if let Some(k) = self.frontend_key.read().await.clone() {
            return Some(k);
        }
        let key = ghost_db::repos::integrations::internal_content_key(&self.db)
            .await
            .ok()
            .flatten()?;
        *self.frontend_key.write().await = Some(key.clone());
        Some(key)
    }

    /// Refresh the setup flag from the DB.
    pub async fn refresh_setup_flag(&self) {
        let done = ghost_db::repos::users::is_setup(&self.db).await.unwrap_or(false);
        self.setup_done.store(done, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn is_setup(&self) -> bool {
        self.setup_done.load(std::sync::atomic::Ordering::Relaxed)
    }
}
