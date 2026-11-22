-- ============================================================================
-- 0030_builder.sql — Tools → Page Builder & Visual Editor
-- ============================================================================
--
-- Three tables and nine settings rows.
--
-- `builder_layouts` — one per post or page that has been opened in the
-- builder. The post's own Lexical content is never touched: while `active`
-- is 1 the site draws `html` instead of it, and switching it off brings the
-- original back.
--   data    the layout JSON, already rebuilt by ghost_core::builder::normalise
--   html    what ghost_core::builder::render made of it, kept so a page view
--           never re-renders; rebuilt on every save and on a settings change
--
-- `builder_revisions` — the previous `data` on every save, newest kept.
--
-- `builder_templates` — sections and whole pages saved for reuse.
--   kind    section | page
--
-- `enabled` ships ON: a site with no layouts looks exactly as it did.
CREATE TABLE IF NOT EXISTS builder_layouts (
  post_id     TEXT PRIMARY KEY REFERENCES posts(id) ON DELETE CASCADE,
  data        TEXT NOT NULL,
  html        TEXT NOT NULL,
  active      INTEGER NOT NULL DEFAULT 1,
  updated_by  TEXT NOT NULL DEFAULT '',
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS builder_revisions (
  id          TEXT PRIMARY KEY,
  post_id     TEXT NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
  data        TEXT NOT NULL,
  created_by  TEXT NOT NULL DEFAULT '',
  created_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS builder_revisions_post_idx ON builder_revisions (post_id, created_at);

CREATE TABLE IF NOT EXISTS builder_templates (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  kind        TEXT NOT NULL DEFAULT 'section',
  data        TEXT NOT NULL,
  created_by  TEXT NOT NULL DEFAULT '',
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);

INSERT OR IGNORE INTO settings (id,`group`,key,value,type,created_at,updated_at) VALUES
(lower(hex(randomblob(12))),'builder','builder_enabled','true','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'builder','builder_post_types','all','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'builder','builder_who','everyone','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'builder','builder_container_width','1140','number',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'builder','builder_section_space','80','number',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'builder','builder_radius','8','number',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'builder','builder_palette',',#7c4dff,#15171a,#6b7280,#f5f5f7,#ffffff','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'builder','builder_heading_font','theme','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'builder','builder_animations','true','boolean',datetime('now'),datetime('now'));
