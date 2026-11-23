-- Repair only the reversed seed rows from 0028 and 0029. Leave correctly
-- stored site settings and all directory content intact.
UPDATE settings
SET value = type,
    type = CASE
        WHEN `key` IN ('directory_classified_days', 'directory_per_page', 'cache_ttl', 'cache_max_pages', 'cache_browser_ttl', 'cache_cdn_ttl')
            THEN 'number'
        WHEN `key` IN ('directory_enabled', 'directory_submissions', 'directory_members_only', 'directory_moderate', 'directory_show_contact', 'cache_pages', 'cache_etag', 'cache_gzip', 'perf_lazy_images', 'cache_cdn_auto_purge')
            THEN 'boolean'
        ELSE 'string'
    END
WHERE `group` IN ('caching', 'directory')
  AND (`key`, value, type) IN (
      ('directory_enabled', 'boolean', 'false'),
      ('directory_title', 'string', ''),
      ('directory_intro', 'string', ''),
      ('directory_submissions', 'boolean', 'false'),
      ('directory_members_only', 'boolean', 'false'),
      ('directory_moderate', 'boolean', 'true'),
      ('directory_classified_days', 'number', '30'),
      ('directory_per_page', 'number', '24'),
      ('directory_show_contact', 'boolean', 'true'),
      ('directory_currency', 'string', 'USD'),
      ('cache_pages', 'boolean', 'false'),
      ('cache_ttl', 'number', '300'),
      ('cache_max_pages', 'number', '500'),
      ('cache_exclude', 'string', ''),
      ('cache_browser_ttl', 'number', '0'),
      ('cache_cdn_ttl', 'number', '0'),
      ('cache_etag', 'boolean', 'false'),
      ('cache_gzip', 'boolean', 'false'),
      ('perf_lazy_images', 'boolean', 'false'),
      ('perf_speculation', 'string', 'off'),
      ('perf_preconnect', 'string', ''),
      ('cache_cdn_auto_purge', 'boolean', 'false')
  );
