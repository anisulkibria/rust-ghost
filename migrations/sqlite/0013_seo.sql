-- ============================================================================
-- 0013_seo.sql — the SEO module's settings
-- ============================================================================
--
-- The module has **no table of its own**. The report it renders is computed
-- from `posts` every time it is asked for, so there is nothing to store and
-- nothing that can go stale; see crates/ghost-db/src/repos/seo.rs for why
-- that is the only honest arrangement. What is here is the seven settings
-- rows the module writes.
--
-- They are rows rather than absent keys because `SettingsCache::set` only
-- ever UPDATEs: a key with no row is refused as "unknown setting", so a site
-- that upgraded into this module would show a settings page whose Save button
-- always failed. The same keys are in `crates/ghost-db/assets/
-- default-settings.json` for a brand-new install; this file is what carries
-- them onto a database that already exists.
--
-- Every default reproduces the behaviour the site had before the module
-- existed, so applying this migration changes nothing on its own:
--
--   seo_structured_data      true   Ghost emits Open Graph, Twitter cards and
--                                   schema.org JSON-LD unless privacy config
--                                   turns structured data off.
--   seo_append_site_title    false  Ghost titles a post with its own title and
--                                   nothing appended. On, it becomes
--                                   "A post - My Site"; archives carry the
--                                   site title either way, which is Ghost's
--                                   own behaviour and not configurable.
--   seo_sitemap_images       true   Feature images are listed in the post and
--                                   page sitemaps, as Ghost does.
--   seo_noindex_tag_pages    false  A tag archive is a legitimate landing
--   seo_noindex_author_pages false  page; hiding one is a choice, not a fix.
--   seo_noindex_paged_pages  false  `follow` is kept in all three cases.
--   seo_referrer_policy      ''     Empty reads as Ghost's default,
--                                   `no-referrer-when-downgrade`, through
--                                   ghost_core::seo::referrer_policy, which
--                                   also refuses any value we do not offer.
--   seo_robots_txt           ''     Empty means "serve the built-in file",
--                                   which names the sitemap and disallows
--                                   only /ghost/ and the machinery. Anything
--                                   stored here replaces it completely.
--
-- `indexnow_api_key` is not here: it exists already from the initial schema.
-- Settings → SEO is simply the first screen that offers it.
--
-- The boolean rows carry type 'boolean' and the two text rows 'string',
-- because `SettingsCache` reads the stored type to decide how to parse the
-- value. A boolean stored as a string still reads correctly —
-- ghost_core::seo::flag accepts "true"/"false" for exactly that reason — but
-- the Admin API serializes settings from this column, so getting it wrong
-- would hand an API client a string where it expects a boolean.

INSERT INTO settings (id, `group`, key, value, type, created_at, updated_at)
SELECT lower(hex(randomblob(12))), 'seo', k, v, 'boolean', datetime('now'), datetime('now')
FROM (
    SELECT 'seo_structured_data'      AS k, 'true'  AS v UNION ALL
    SELECT 'seo_append_site_title',         'false'      UNION ALL
    SELECT 'seo_sitemap_images',            'true'       UNION ALL
    SELECT 'seo_noindex_tag_pages',         'false'      UNION ALL
    SELECT 'seo_noindex_author_pages',      'false'      UNION ALL
    SELECT 'seo_noindex_paged_pages',       'false'
) AS defaults
WHERE NOT EXISTS (SELECT 1 FROM settings s WHERE s.key = defaults.k);

INSERT INTO settings (id, `group`, key, value, type, created_at, updated_at)
SELECT lower(hex(randomblob(12))), 'seo', k, '', 'string', datetime('now'), datetime('now')
FROM (
    SELECT 'seo_referrer_policy' AS k UNION ALL
    SELECT 'seo_robots_txt'
) AS defaults
WHERE NOT EXISTS (SELECT 1 FROM settings s WHERE s.key = defaults.k);
