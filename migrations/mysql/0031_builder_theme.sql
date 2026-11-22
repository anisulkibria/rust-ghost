-- ============================================================================
-- 0031_builder_theme.sql — Page Builder: Theme Builder, global widgets, kit
-- ============================================================================
--
-- MySQL twin of migrations/sqlite/0031_builder_theme.sql; see there.
-- `options` is VARCHAR (read as a plain string); `html` is MEDIUMTEXT, read
-- through ghost_db::types::Text, and nullable because MySQL gives a
-- MEDIUMTEXT no literal default.
ALTER TABLE builder_templates ADD COLUMN options VARCHAR(4000) NOT NULL DEFAULT '{}';
ALTER TABLE builder_templates ADD COLUMN active SMALLINT NOT NULL DEFAULT 0;
ALTER TABLE builder_templates ADD COLUMN html MEDIUMTEXT NULL;
CREATE INDEX builder_templates_kind_idx ON builder_templates (kind, active);

INSERT IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(RANDOM_BYTES(12))),'builder','builder_body_font','theme','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'builder','builder_google_fonts','true','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'builder','builder_custom_font_name','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'builder','builder_custom_font_url','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'builder','builder_theme_header_selector','#gh-head, .gh-head, .site-header, body > header, body > nav','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'builder','builder_theme_footer_selector','.gh-foot, .gh-footer, .site-footer, body > footer','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'builder','builder_maintenance','off','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'builder','builder_maintenance_page','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP());
