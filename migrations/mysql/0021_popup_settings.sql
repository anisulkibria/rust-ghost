-- ============================================================================
-- 0021_popup_settings.sql — Settings → Popups
-- ============================================================================
--
-- The MySQL twin of migrations/sqlite/0021_popup_settings.sql; read that one
-- for why each default reproduces the module as it was.
INSERT IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(RANDOM_BYTES(12))),'popups','popups_enabled','true','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'popups','popups_hide_on_mobile','false','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'popups','popups_cooldown_hours','0','number',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'popups','popups_consent_text','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP());
