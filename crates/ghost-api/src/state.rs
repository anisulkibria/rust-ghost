//! Shared application state.
use ghost_core::Config;
use ghost_db::SettingsCache;
use ghost_render::ThemeState;
use ghost_routing::redirects::Redirects;
use ghost_routing::RouterTree;
use ghost_db::Db;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
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
    /// Ceilings the host has put on this site, read once from
    /// `hostSettings.limits`. Empty on a self-hosted install, where every
    /// check is a map lookup that finds nothing — see [`ghost_db::limits`].
    ///
    /// Resolved at boot rather than per request because it is config: it
    /// cannot change while the process runs, and the checks sit on paths
    /// (every API key authentication, every upload) where re-parsing it would
    /// be pure waste.
    pub limits: Arc<ghost_db::limits::Limits>,
    /// The firewall's rules, in memory.
    ///
    /// Unlike [`Self::limits`] this *can* change while the process runs — the
    /// Security screen writes rules — so it is a cache with a `reload` rather
    /// than a value resolved once. It is here, on the state, because the
    /// middleware that reads it runs before any handler and on every request,
    /// including the requests it is about to refuse; a query there would mean
    /// a database round trip per request at exactly the moment a site is
    /// under the load that makes a firewall worth having.
    ///
    /// Empty by default, so a test or an embedder that builds an `AppState`
    /// by hand gets a firewall that lets everybody through rather than one
    /// that refuses everybody. See [`ghost_db::repos::firewall::RuleCache`].
    pub firewall: ghost_db::repos::firewall::RuleCache,
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
