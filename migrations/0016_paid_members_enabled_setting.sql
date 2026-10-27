-- `paid_members_enabled` had no row to update.
--
-- Connecting Stripe writes this flag, and `SettingsCache::set` is an UPDATE
-- that errors on an unknown key — so the write silently never landed and the
-- flag stayed false on every install. Portal reads it to decide whether paid
-- membership exists at all, which made `#/portal/signup/monthly`,
-- `#/portal/signup/<tier>/<cadence>`, `#/portal/account/plans`,
-- `#/portal/gift` and `#/portal/support` all refuse to open even with Stripe
-- connected and tiers priced.
--
-- Seeded from the connection that is already there, rather than false: a site
-- that connected Stripe before this migration should not have to reconnect.
INSERT INTO settings (id, `group`, key, value, type, created_at, updated_at)
SELECT lower(hex(randomblob(12))), 'members', 'paid_members_enabled',
       CASE WHEN EXISTS (
           SELECT 1 FROM settings
           WHERE key IN ('stripe_secret_key', 'stripe_connect_secret_key')
             AND value IS NOT NULL AND trim(value) <> ''
       ) THEN 'true' ELSE 'false' END,
       'boolean', datetime('now'), datetime('now')
WHERE NOT EXISTS (SELECT 1 FROM settings WHERE key = 'paid_members_enabled');
