-- The Stripe price a tip lands on. Ghost caches its donation price in the
-- `stripe_prices` table; this port has no such table, so the ids live in
-- settings where SettingsCache can read them without a query.
--
-- `SettingsCache::set` is an UPDATE, not an upsert — an unknown key is a
-- hard error — so every key the checkout writes must exist up front.
INSERT INTO settings (id, `group`, key, value, type, created_at, updated_at)
SELECT lower(hex(randomblob(12))), g, k, v, 'string', datetime('now'), datetime('now')
FROM (
    SELECT 'members' AS g, 'donations_product_id' AS k, '' AS v UNION ALL
    SELECT 'members',       'donations_price_id',         ''
) AS defaults
WHERE NOT EXISTS (SELECT 1 FROM settings s WHERE s.key = defaults.k);
