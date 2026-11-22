-- ============================================================================
-- 0019_popups.sql — popups, and the leads they capture
-- ============================================================================
--
-- The MySQL twin of migrations/sqlite/0019_popups.sql; read that one for why
-- the counters live on the popup row and why `member_id` is nullable and not
-- a foreign key.
--
-- Every column that SQLite leaves as unbounded TEXT is sized here, because a
-- MySQL index cannot cover an unbounded column and because `paths` and
-- `body` are the only two that need room. Both stay inside the row rather
-- than becoming MEDIUMTEXT, so nothing here has to be read through
-- `ghost_db::types::Text`.
CREATE TABLE IF NOT EXISTS popups (
  id              VARCHAR(24) PRIMARY KEY,
  name            VARCHAR(191) NOT NULL,
  status          VARCHAR(20) NOT NULL DEFAULT 'draft',
  kind            VARCHAR(20) NOT NULL DEFAULT 'modal',
  heading         VARCHAR(191) NOT NULL DEFAULT '',
  body            VARCHAR(1000) NOT NULL DEFAULT '',
  button_label    VARCHAR(100) NOT NULL DEFAULT '',
  button_url      VARCHAR(500) NOT NULL DEFAULT '',
  success_message VARCHAR(500) NOT NULL DEFAULT '',
  -- SMALLINT, not TINYINT/BOOLEAN: sqlx's `Any` driver maps only SMALLINT,
  -- INT and BIGINT from MySQL, and a TINYINT column fails at READ time —
  -- the migration applies and the insert succeeds, and the first page that
  -- reads it breaks. Read as i64 and compared `!= 0`.
  capture_email   SMALLINT NOT NULL DEFAULT 1,
  capture_name    SMALLINT NOT NULL DEFAULT 0,
  trigger_kind    VARCHAR(20) NOT NULL DEFAULT 'delay',
  trigger_value   INT NOT NULL DEFAULT 5,
  frequency       VARCHAR(20) NOT NULL DEFAULT 'once',
  frequency_days  INT NOT NULL DEFAULT 0,
  audience        VARCHAR(20) NOT NULL DEFAULT 'logged_out',
  targeting       VARCHAR(20) NOT NULL DEFAULT 'everywhere',
  paths           VARCHAR(4000) NOT NULL DEFAULT '',
  starts_at       VARCHAR(32) NULL,
  ends_at         VARCHAR(32) NULL,
  impressions     BIGINT NOT NULL DEFAULT 0,
  submissions     BIGINT NOT NULL DEFAULT 0,
  created_at      VARCHAR(32) NOT NULL,
  updated_at      VARCHAR(32) NOT NULL,
  INDEX popups_status_idx (status)
);

CREATE TABLE IF NOT EXISTS popup_leads (
  id         VARCHAR(24) PRIMARY KEY,
  popup_id   VARCHAR(24) NOT NULL,
  email      VARCHAR(191) NOT NULL,
  name       VARCHAR(191) NOT NULL DEFAULT '',
  source_url VARCHAR(500) NOT NULL DEFAULT '',
  member_id  VARCHAR(24) NULL,
  created_at VARCHAR(32) NOT NULL,
  INDEX popup_leads_popup_idx (popup_id, created_at),
  INDEX popup_leads_email_idx (popup_id, email)
);
