-- ============================================================================
-- 0022_image_settings.sql — Settings → Images
-- ============================================================================
--
-- The Media module's optimisation settings, as rows so `SettingsCache::set`
-- has something to UPDATE.
--
--   images_optimize        false  the master switch. OFF, because turning it
--                                 on rewrites people's uploads, and that is a
--                                 decision somebody makes — not something an
--                                 upgrade does to them. With it off, uploads
--                                 are stored byte-for-byte as before.
--   images_max_dimension   2000   the longest side kept; Ghost's own figure
--   images_jpeg_quality    82     indistinguishable on screen, ~1/3 the bytes
--   images_strip_metadata  true   camera metadata, including GPS, removed
--
-- The last three only take effect once the first is on, and are the values
-- the settings screen recommends, so switching optimisation on does something
-- sensible without further choices.
INSERT OR IGNORE INTO settings (id,`group`,key,value,type,created_at,updated_at) VALUES
(lower(hex(randomblob(12))),'images','images_optimize','false','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'images','images_max_dimension','2000','number',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'images','images_jpeg_quality','82','number',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'images','images_strip_metadata','true','boolean',datetime('now'),datetime('now'));
