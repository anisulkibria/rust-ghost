-- ============================================================================
-- 0026_workflows.sql — Tools → Workflow & Editorial Management
-- ============================================================================
--
-- MySQL twin of migrations/sqlite/0026_workflows.sql.
--
-- Four tables and five settings rows. Nothing here duplicates a post: every
-- table is keyed by `post_id` and says only what the post itself cannot —
-- where it is in the editorial pipeline and what people said about it.
--
-- `workflow_items` — one row per post or page that has been placed on the
-- board. A draft with no row sits in the second stage (writing); scheduled
-- and published entries are shown by their own status, not by `stage`.
--   stage      a key from the `workflow_stages` setting
--   due_on     YYYY-MM-DD, or NULL
--   priority   low | normal | high | urgent
--
-- `workflow_checks` — the manual checklist's ticks, one row per item ticked.
--   check_key  a slug of the checklist line, so renaming a line resets it
--
-- `workflow_notes` — editorial notes: internal, never on the site.
--
-- `workflow_events` — the item's history: created, moved, assigned, noted.
--   kind       created | moved | assigned | due | priority | checked | note
--
-- The settings ship with the approval gate OFF: installing this changes no
-- one's ability to publish.
CREATE TABLE IF NOT EXISTS workflow_items (
  post_id     VARCHAR(24) PRIMARY KEY,
  stage       VARCHAR(30) NOT NULL,
  assignee_id VARCHAR(24) NULL,
  reviewer_id VARCHAR(24) NULL,
  due_on      VARCHAR(10) NULL,
  priority    VARCHAR(10) NOT NULL DEFAULT 'normal',
  updated_at  VARCHAR(32) NOT NULL,
  INDEX workflow_items_assignee_idx (assignee_id),
  INDEX workflow_items_due_idx (due_on)
);

CREATE TABLE IF NOT EXISTS workflow_checks (
  post_id   VARCHAR(24) NOT NULL,
  check_key VARCHAR(80) NOT NULL,
  done_by   VARCHAR(24) NOT NULL DEFAULT '',
  done_at   VARCHAR(32) NOT NULL,
  PRIMARY KEY (post_id, check_key)
);

CREATE TABLE IF NOT EXISTS workflow_notes (
  id         VARCHAR(24) PRIMARY KEY,
  post_id    VARCHAR(24) NOT NULL,
  author_id  VARCHAR(24) NOT NULL,
  body       VARCHAR(2000) NOT NULL,
  created_at VARCHAR(32) NOT NULL,
  INDEX workflow_notes_post_idx (post_id, created_at)
);

CREATE TABLE IF NOT EXISTS workflow_events (
  id         VARCHAR(24) PRIMARY KEY,
  post_id    VARCHAR(24) NOT NULL,
  actor_id   VARCHAR(24) NOT NULL,
  kind       VARCHAR(20) NOT NULL,
  detail     VARCHAR(300) NOT NULL DEFAULT '',
  created_at VARCHAR(32) NOT NULL,
  INDEX workflow_events_post_idx (post_id, created_at)
);

INSERT IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(RANDOM_BYTES(12))),'workflows','workflow_stages','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'workflows','workflow_require_approval','false','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'workflows','workflow_approval_stage','ready','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'workflows','workflow_admins_bypass','true','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'workflows','workflow_checklist','Facts checked\nLinks checked\nRead aloud once','string',UTC_TIMESTAMP(),UTC_TIMESTAMP());
