-- ============================================================================
-- 0028_caching.sql — Tools → Caching & Performance
-- ============================================================================
--
-- MySQL twin of migrations/sqlite/0028_caching.sql.
--
-- Twelve settings rows and one table.
--
-- The page cache itself is in memory, not here: a cached page is worth
-- keeping only as long as the process that rendered it, and a restart is
-- exactly when it should be empty. What is stored is the configuration —
-- every switch OFF, so installing this changes no response — and the CDN
-- connection.
--
-- `cdn_config` — one row (`id = 'default'`) for the CDN the site sits
-- behind, so the tool can tell it to forget pages when content changes.
--   provider  cloudflare (the only one, for now)
--   zone      the Cloudflare zone id
--   token     an API token with Zone → Cache Purge. A secret: kept here and
--             NOT in `settings`, which the settings APIs serialize. Never
--             sent back to a browser.
CREATE TABLE IF NOT EXISTS cdn_config (
  id         VARCHAR(24) PRIMARY KEY,
  provider   VARCHAR(20) NOT NULL DEFAULT 'cloudflare',
  zone       VARCHAR(64) NOT NULL DEFAULT '',
  token      VARCHAR(200) NOT NULL DEFAULT '',
  last_purge VARCHAR(32) NULL,
  last_error VARCHAR(300) NOT NULL DEFAULT '',
  updated_at VARCHAR(32) NOT NULL
);

INSERT IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(RANDOM_BYTES(12))),'caching','cache_pages','boolean','false',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'caching','cache_ttl','number','300',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'caching','cache_max_pages','number','500',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'caching','cache_exclude','string','',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'caching','cache_browser_ttl','number','0',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'caching','cache_cdn_ttl','number','0',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'caching','cache_etag','boolean','false',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'caching','cache_gzip','boolean','false',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'caching','perf_lazy_images','boolean','false',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'caching','perf_speculation','string','off',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'caching','perf_preconnect','string','',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'caching','cache_cdn_auto_purge','boolean','false',UTC_TIMESTAMP(),UTC_TIMESTAMP());
