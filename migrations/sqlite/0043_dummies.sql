-- ============================================================================
-- 0043_dummies.sql — Tools → Dummy content: what it generated
-- ============================================================================
--
-- Dummy content is ordinary content (posts, tags, users, members, products,
-- menu links) made for demonstration. These two tables are the only record of
-- which rows are dummies, so they can be removed again without touching
-- anything a person wrote. Nothing else reads them.
--
-- `dummy_batches` — one generation request.
--   `label`      'full' (the one-click demo) or 'custom'.
--   `summary`    what was asked for, in words ("6 tags and 16 posts").
--   `created_by` the staff user who generated it.
--
-- `dummy_items` — one generated thing.
--   `kind`        tags | users | posts | pages | members | products | navigation
--   `item_id`     the row's id; for a menu link, `<setting key>|<url>`.
--   `fingerprint` the row's `updated_at` when it was made (for a menu link,
--                 its label). Removal deletes an item only while this still
--                 matches: anything edited since has become somebody's work.
CREATE TABLE IF NOT EXISTS dummy_batches (
  id          TEXT PRIMARY KEY,
  label       TEXT NOT NULL,
  summary     TEXT NOT NULL,
  created_by  TEXT,
  created_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS dummy_items (
  id          TEXT PRIMARY KEY,
  batch_id    TEXT NOT NULL,
  kind        TEXT NOT NULL,
  item_id     TEXT NOT NULL,
  fingerprint TEXT NOT NULL,
  created_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS dummy_items_batch_idx ON dummy_items (batch_id);
CREATE INDEX IF NOT EXISTS dummy_items_kind_idx ON dummy_items (kind, item_id);
