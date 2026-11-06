-- Checkout keeps an owner-facing setup diagnostic while Portal receives a
-- message it knows how to translate. SettingsCache only updates known keys.
INSERT INTO settings (id, `group`, `key`, value, type, created_at, updated_at)
SELECT lower(hex(RANDOM_BYTES(12))), 'members', 'stripe_checkout_setup_error', '', 'string', UTC_TIMESTAMP(), UTC_TIMESTAMP()
WHERE NOT EXISTS (SELECT 1 FROM settings WHERE `key` = 'stripe_checkout_setup_error');
