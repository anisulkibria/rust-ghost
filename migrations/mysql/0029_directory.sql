-- ============================================================================
-- 0029_directory.sql — Tools → Directory & Classified Listings
-- ============================================================================
--
-- MySQL twin of migrations/sqlite/0029_directory.sql.
--
-- Two tables and ten settings rows.
--
-- `directory_categories` — how listings are grouped on the public directory.
--   tint      one of the admin's tints, for the category's chip
--
-- `directory_listings` — one entry. Directory entries and classifieds share
-- the table; a classified has a price and an `expires_at`.
--   kind        directory | classified
--   status      pending | published | rejected | draft
--   description plain text; never stored as HTML. Blank lines become
--               paragraphs when it is shown, escaped.
--   price_type  none | fixed | negotiable | free | on_request
--   submitter_* who sent it from the public form (blank for staff entries).
--               Personal data: shown only to staff, never on the site.
--   views, contact_clicks  running counts, approximate by design
--
-- The settings ship with the public directory OFF and submissions OFF:
-- installing this adds no page to the site.
CREATE TABLE IF NOT EXISTS directory_categories (
  id          VARCHAR(24) PRIMARY KEY,
  name        VARCHAR(80) NOT NULL,
  slug        VARCHAR(80) NOT NULL UNIQUE,
  description VARCHAR(300) NOT NULL DEFAULT '',
  tint        VARCHAR(20) NOT NULL DEFAULT 'blue',
  sort_order  INT NOT NULL DEFAULT 0,
  created_at  VARCHAR(32) NOT NULL,
  updated_at  VARCHAR(32) NOT NULL
);

CREATE TABLE IF NOT EXISTS directory_listings (
  id              VARCHAR(24) PRIMARY KEY,
  slug            VARCHAR(100) NOT NULL UNIQUE,
  title           VARCHAR(120) NOT NULL,
  kind            VARCHAR(20) NOT NULL DEFAULT 'directory',
  status          VARCHAR(20) NOT NULL DEFAULT 'pending',
  category_id     VARCHAR(24) NULL,
  summary         VARCHAR(240) NOT NULL DEFAULT '',
  -- VARCHAR, not TEXT: sqlx `Any` cannot read TEXT back from MySQL.
  description     VARCHAR(5000) NOT NULL DEFAULT '',
  image           VARCHAR(500) NOT NULL DEFAULT '',
  website         VARCHAR(500) NOT NULL DEFAULT '',
  email           VARCHAR(191) NOT NULL DEFAULT '',
  phone           VARCHAR(40) NOT NULL DEFAULT '',
  address         VARCHAR(120) NOT NULL DEFAULT '',
  location        VARCHAR(80) NOT NULL DEFAULT '',
  price_cents     BIGINT NOT NULL DEFAULT 0,
  price_type      VARCHAR(20) NOT NULL DEFAULT 'none',
  keywords        VARCHAR(200) NOT NULL DEFAULT '',
  featured        SMALLINT NOT NULL DEFAULT 0,
  submitter_name  VARCHAR(80) NOT NULL DEFAULT '',
  submitter_email VARCHAR(191) NOT NULL DEFAULT '',
  member_id       VARCHAR(24) NULL,
  reject_reason   VARCHAR(300) NOT NULL DEFAULT '',
  views           BIGINT NOT NULL DEFAULT 0,
  contact_clicks  BIGINT NOT NULL DEFAULT 0,
  expires_at      VARCHAR(32) NULL,
  published_at    VARCHAR(32) NULL,
  created_at      VARCHAR(32) NOT NULL,
  updated_at      VARCHAR(32) NOT NULL,
  INDEX directory_listings_status_idx (status, published_at),
  INDEX directory_listings_category_idx (category_id),
  INDEX directory_listings_submitter_idx (submitter_email, created_at)
);

INSERT IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(RANDOM_BYTES(12))),'directory','directory_enabled','boolean','false',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'directory','directory_title','string','',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'directory','directory_intro','string','',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'directory','directory_submissions','boolean','false',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'directory','directory_members_only','boolean','false',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'directory','directory_moderate','boolean','true',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'directory','directory_classified_days','number','30',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'directory','directory_per_page','number','24',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'directory','directory_show_contact','boolean','true',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'directory','directory_currency','string','USD',UTC_TIMESTAMP(),UTC_TIMESTAMP());
