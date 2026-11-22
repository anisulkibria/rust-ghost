-- ============================================================================
-- 0031_builder_theme.sql — Page Builder: Theme Builder, global widgets, kit
-- ============================================================================
--
-- `builder_templates` becomes the home of every reusable layout: saved
-- sections and pages, global widgets, and the Theme Builder's headers,
-- footers, single-post templates and popups.
--   options   JSON: where a part applies, and how a popup opens
--             (ghost_core::builder::parts::PartOptions)
--   active    a Theme Builder part only runs while this is 1
--   html      the rendered layout, kept so a page view never re-renders
--
-- Eight new settings rows: fonts, the theme header/footer selectors the
-- Theme Builder hides, and maintenance mode (off by default).
ALTER TABLE builder_templates ADD COLUMN options TEXT NOT NULL DEFAULT '{}';
ALTER TABLE builder_templates ADD COLUMN active INTEGER NOT NULL DEFAULT 0;
ALTER TABLE builder_templates ADD COLUMN html TEXT NOT NULL DEFAULT '';
CREATE INDEX IF NOT EXISTS builder_templates_kind_idx ON builder_templates (kind, active);

INSERT OR IGNORE INTO settings (id,`group`,key,value,type,created_at,updated_at) VALUES
(lower(hex(randomblob(12))),'builder','builder_body_font','theme','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'builder','builder_google_fonts','true','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'builder','builder_custom_font_name','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'builder','builder_custom_font_url','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'builder','builder_theme_header_selector','#gh-head, .gh-head, .site-header, body > header, body > nav','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'builder','builder_theme_footer_selector','.gh-foot, .gh-footer, .site-footer, body > footer','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'builder','builder_maintenance','off','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'builder','builder_maintenance_page','','string',datetime('now'),datetime('now'));
