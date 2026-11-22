-- ============================================================================
-- 0030_builder.sql — Tools → Page Builder & Visual Editor
-- ============================================================================
--
-- MySQL twin of migrations/sqlite/0030_builder.sql; see there for the why.
-- `data` and `html` are MEDIUMTEXT (a layout outgrows any VARCHAR) and are
-- read through ghost_db::types::Text, which takes MySQL's blob form.
CREATE TABLE IF NOT EXISTS builder_layouts (
  post_id     VARCHAR(191) PRIMARY KEY,
  data        MEDIUMTEXT NOT NULL,
  html        MEDIUMTEXT NOT NULL,
  active      SMALLINT NOT NULL DEFAULT 1,
  updated_by  VARCHAR(24) NOT NULL DEFAULT '',
  created_at  VARCHAR(32) NOT NULL,
  updated_at  VARCHAR(32) NOT NULL,
  CONSTRAINT builder_layouts_post_fk FOREIGN KEY (post_id) REFERENCES posts(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS builder_revisions (
  id          VARCHAR(24) PRIMARY KEY,
  post_id     VARCHAR(191) NOT NULL,
  data        MEDIUMTEXT NOT NULL,
  created_by  VARCHAR(24) NOT NULL DEFAULT '',
  created_at  VARCHAR(32) NOT NULL,
  CONSTRAINT builder_revisions_post_fk FOREIGN KEY (post_id) REFERENCES posts(id) ON DELETE CASCADE
);
CREATE INDEX builder_revisions_post_idx ON builder_revisions (post_id, created_at);

CREATE TABLE IF NOT EXISTS builder_templates (
  id          VARCHAR(24) PRIMARY KEY,
  name        VARCHAR(120) NOT NULL,
  kind        VARCHAR(16) NOT NULL DEFAULT 'section',
  data        MEDIUMTEXT NOT NULL,
  created_by  VARCHAR(24) NOT NULL DEFAULT '',
  created_at  VARCHAR(32) NOT NULL,
  updated_at  VARCHAR(32) NOT NULL
);

INSERT IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(RANDOM_BYTES(12))),'builder','builder_enabled','true','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'builder','builder_post_types','all','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'builder','builder_who','everyone','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'builder','builder_container_width','1140','number',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'builder','builder_section_space','80','number',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'builder','builder_radius','8','number',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'builder','builder_palette',',#7c4dff,#15171a,#6b7280,#f5f5f7,#ffffff','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'builder','builder_heading_font','theme','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'builder','builder_animations','true','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP());
