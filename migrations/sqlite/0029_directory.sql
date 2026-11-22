-- ============================================================================
-- 0029_directory.sql — Tools → Directory & Classified Listings
-- ============================================================================
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
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  slug        TEXT NOT NULL UNIQUE,
  description TEXT NOT NULL DEFAULT '',
  tint        TEXT NOT NULL DEFAULT 'blue',
  sort_order  INTEGER NOT NULL DEFAULT 0,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS directory_listings (
  id              TEXT PRIMARY KEY,
  slug            TEXT NOT NULL UNIQUE,
  title           TEXT NOT NULL,
  kind            TEXT NOT NULL DEFAULT 'directory',
  status          TEXT NOT NULL DEFAULT 'pending',
  category_id     TEXT,
  summary         TEXT NOT NULL DEFAULT '',
  description     TEXT NOT NULL DEFAULT '',
  image           TEXT NOT NULL DEFAULT '',
  website         TEXT NOT NULL DEFAULT '',
  email           TEXT NOT NULL DEFAULT '',
  phone           TEXT NOT NULL DEFAULT '',
  address         TEXT NOT NULL DEFAULT '',
  location        TEXT NOT NULL DEFAULT '',
  price_cents     INTEGER NOT NULL DEFAULT 0,
  price_type      TEXT NOT NULL DEFAULT 'none',
  keywords        TEXT NOT NULL DEFAULT '',
  featured        INTEGER NOT NULL DEFAULT 0,
  submitter_name  TEXT NOT NULL DEFAULT '',
  submitter_email TEXT NOT NULL DEFAULT '',
  member_id       TEXT,
  reject_reason   TEXT NOT NULL DEFAULT '',
  views           INTEGER NOT NULL DEFAULT 0,
  contact_clicks  INTEGER NOT NULL DEFAULT 0,
  expires_at      TEXT,
  published_at    TEXT,
  created_at      TEXT NOT NULL,
  updated_at      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS directory_listings_status_idx ON directory_listings (status, published_at);
CREATE INDEX IF NOT EXISTS directory_listings_category_idx ON directory_listings (category_id);
CREATE INDEX IF NOT EXISTS directory_listings_submitter_idx ON directory_listings (submitter_email, created_at);

INSERT OR IGNORE INTO settings (id,`group`,key,value,type,created_at,updated_at) VALUES
(lower(hex(randomblob(12))),'directory','directory_enabled','boolean','false',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'directory','directory_title','string','',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'directory','directory_intro','string','',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'directory','directory_submissions','boolean','false',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'directory','directory_members_only','boolean','false',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'directory','directory_moderate','boolean','true',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'directory','directory_classified_days','number','30',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'directory','directory_per_page','number','24',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'directory','directory_show_contact','boolean','true',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'directory','directory_currency','string','USD',datetime('now'),datetime('now'));
