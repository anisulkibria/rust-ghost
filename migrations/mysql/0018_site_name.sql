-- Keep the short publication name separate from its title. A blank name
-- falls back to the current title in themes and the admin UI.
INSERT INTO settings (id, `group`, `key`, value, type, created_at, updated_at)
SELECT lower(hex(RANDOM_BYTES(12))), 'site', 'name', '', 'string', UTC_TIMESTAMP(), UTC_TIMESTAMP()
WHERE NOT EXISTS (SELECT 1 FROM settings WHERE `key` = 'name');
