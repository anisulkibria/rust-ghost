-- ============================================================================
-- 0023_cookies.sql — Cookie consent & privacy compliance
-- ============================================================================
--
-- The MySQL twin of migrations/sqlite/0023_cookies.sql; read that one for
-- what each column holds and why the log stores no address or user agent.
-- VARCHAR throughout (sqlx `Any` cannot read TEXT back from MySQL) and
-- SMALLINT for the one flag (it cannot read TINYINT either).
CREATE TABLE IF NOT EXISTS cookie_declarations (
  id          VARCHAR(24) PRIMARY KEY,
  name        VARCHAR(100) NOT NULL,
  provider    VARCHAR(100) NOT NULL DEFAULT '',
  category    VARCHAR(20) NOT NULL DEFAULT 'necessary',
  duration    VARCHAR(40) NOT NULL DEFAULT '',
  purpose     VARCHAR(300) NOT NULL DEFAULT '',
  tracker_key VARCHAR(40) NOT NULL DEFAULT '',
  created_at  VARCHAR(32) NOT NULL,
  updated_at  VARCHAR(32) NOT NULL
);

CREATE TABLE IF NOT EXISTS cookie_consents (
  id         VARCHAR(24) PRIMARY KEY,
  consent_id VARCHAR(36) NOT NULL,
  action     VARCHAR(20) NOT NULL,
  categories VARCHAR(80) NOT NULL,
  revision   INT NOT NULL DEFAULT 1,
  gpc        SMALLINT NOT NULL DEFAULT 0,
  page       VARCHAR(300) NOT NULL DEFAULT '',
  created_at VARCHAR(32) NOT NULL,
  INDEX cookie_consents_created_idx (created_at),
  INDEX cookie_consents_consent_idx (consent_id)
);

INSERT IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(RANDOM_BYTES(12))),'cookies','cookies_enabled','false','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'cookies','cookies_mode','opt_in','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'cookies','cookies_layout','box','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'cookies','cookies_title','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'cookies','cookies_message','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'cookies','cookies_policy_url','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'cookies','cookies_revision','1','number',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'cookies','cookies_expiry_months','12','number',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'cookies','cookies_honour_gpc','true','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'cookies','cookies_block_embeds','false','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'cookies','cookies_reopen','true','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'cookies','cookies_log','true','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'cookies','cookies_log_days','365','number',UTC_TIMESTAMP(),UTC_TIMESTAMP());
