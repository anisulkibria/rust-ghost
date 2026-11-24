-- ============================================================================
-- 0043_dummies.sql — Tools → Dummy content: what it generated
-- ============================================================================
--
-- MySQL twin of migrations/sqlite/0043_dummies.sql (see there).
-- VARCHAR, not TEXT: the Any driver cannot read TEXT back on MySQL.
CREATE TABLE IF NOT EXISTS dummy_batches (
  id          VARCHAR(24) PRIMARY KEY,
  label       VARCHAR(20) NOT NULL,
  summary     VARCHAR(300) NOT NULL,
  created_by  VARCHAR(24) NULL,
  created_at  VARCHAR(32) NOT NULL
);

CREATE TABLE IF NOT EXISTS dummy_items (
  id          VARCHAR(24) PRIMARY KEY,
  batch_id    VARCHAR(24) NOT NULL,
  kind        VARCHAR(20) NOT NULL,
  item_id     VARCHAR(600) NOT NULL,
  fingerprint VARCHAR(191) NOT NULL,
  created_at  VARCHAR(32) NOT NULL
);
CREATE INDEX dummy_items_batch_idx ON dummy_items (batch_id);
CREATE INDEX dummy_items_kind_idx ON dummy_items (kind, item_id(191));
