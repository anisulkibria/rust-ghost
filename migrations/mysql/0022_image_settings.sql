-- ============================================================================
-- 0022_image_settings.sql — Settings → Images
-- ============================================================================
--
-- The MySQL twin of migrations/sqlite/0022_image_settings.sql; read that one
-- for why optimisation ships OFF while its three details ship at the values
-- the screen recommends.
INSERT IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(RANDOM_BYTES(12))),'images','images_optimize','false','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'images','images_max_dimension','2000','number',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'images','images_jpeg_quality','82','number',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'images','images_strip_metadata','true','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP());
