-- ============================================================================
-- 0021_popup_settings.sql — Settings → Popups
-- ============================================================================
--
-- The Popups module's four site-wide settings. They have to exist as rows
-- before they can be written: `SettingsCache::set` only ever UPDATEs, and a
-- module with settings and no migration has a Save button that always fails.
--
-- Every value reproduces the module as it behaved before these settings
-- existed, so applying this changes nothing a visitor sees:
--
--   popups_enabled         true   the master switch — popups were always on
--   popups_hide_on_mobile  false  they always showed on phones
--   popups_cooldown_hours  0      no spacing between popups beyond each
--                                 popup's own frequency
--   popups_consent_text    ''     no note under the address field
--
-- `ghost_core::popups::SiteSettings::read` also treats a MISSING row as these
-- defaults, so a site that has not run this yet still shows its popups rather
-- than reading the absent master switch as "off".
INSERT OR IGNORE INTO settings (id,`group`,key,value,type,created_at,updated_at) VALUES
(lower(hex(randomblob(12))),'popups','popups_enabled','true','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'popups','popups_hide_on_mobile','false','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'popups','popups_cooldown_hours','0','number',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'popups','popups_consent_text','','string',datetime('now'),datetime('now'));
