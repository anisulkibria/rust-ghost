-- ============================================================================
-- 0026_workflows.sql — Tools → Workflow & Editorial Management
-- ============================================================================
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
  post_id     TEXT PRIMARY KEY,
  stage       TEXT NOT NULL,
  assignee_id TEXT,
  reviewer_id TEXT,
  due_on      TEXT,
  priority    TEXT NOT NULL DEFAULT 'normal',
  updated_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS workflow_items_assignee_idx ON workflow_items (assignee_id);
CREATE INDEX IF NOT EXISTS workflow_items_due_idx ON workflow_items (due_on);

CREATE TABLE IF NOT EXISTS workflow_checks (
  post_id   TEXT NOT NULL,
  check_key TEXT NOT NULL,
  done_by   TEXT NOT NULL DEFAULT '',
  done_at   TEXT NOT NULL,
  PRIMARY KEY (post_id, check_key)
);

CREATE TABLE IF NOT EXISTS workflow_notes (
  id         TEXT PRIMARY KEY,
  post_id    TEXT NOT NULL,
  author_id  TEXT NOT NULL,
  body       TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS workflow_notes_post_idx ON workflow_notes (post_id, created_at);

CREATE TABLE IF NOT EXISTS workflow_events (
  id         TEXT PRIMARY KEY,
  post_id    TEXT NOT NULL,
  actor_id   TEXT NOT NULL,
  kind       TEXT NOT NULL,
  detail     TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS workflow_events_post_idx ON workflow_events (post_id, created_at);

INSERT OR IGNORE INTO settings (id,`group`,key,value,type,created_at,updated_at) VALUES
(lower(hex(randomblob(12))),'workflows','workflow_stages','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'workflows','workflow_require_approval','false','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'workflows','workflow_approval_stage','ready','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'workflows','workflow_admins_bypass','true','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'workflows','workflow_checklist','Facts checked\nLinks checked\nRead aloud once','string',datetime('now'),datetime('now'));
