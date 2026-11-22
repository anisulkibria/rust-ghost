-- ============================================================================
-- 0025_ads.sql — Tools → Advertising & Ad Management
-- ============================================================================
--
-- MySQL twin of migrations/sqlite/0025_ads.sql.
--
-- Three tables and five settings rows.
--
-- `ad_advertisers` — who an ad is for: the name on the report and an address
-- to send it to. An ad need not have one (house ads promote the site itself).
--
-- `ads` — one creative and everything that decides where it runs.
--   status     draft | live | paused
--   kind       image | text | code      (code = an ad network's tag)
--   placement  post_top | post_paragraph | post_bottom | sticky | slot
--   audience   everyone | visitors | free | not_paid
--   device     all | desktop | mobile   (decided in CSS, not by sniffing)
--   targeting  everywhere | only | except, over `rules`: one path prefix or
--              `tag:slug` per line
--   weight     relative share when several ads compete for one placement
--   max_*      caps; 0 means none. Checked against the running totals below.
--   cpm/cpc    prices in cents, for the revenue estimate only
--   impressions, clicks  running totals, so a cap is one column read rather
--              than a SUM over the daily table on every page view
--
-- `ad_daily` — the same two counts per ad per day, for the charts and the
-- advertiser report. One row per (ad, day), incremented in place.
--
-- An impression is counted by the visitor script once half the ad has been
-- on screen for a second (the IAB viewability rule), not when the page was
-- rendered, so a report does not sell ads nobody could have seen.
--
-- The settings ship OFF: no ad anywhere, no script, and /ads.txt keeps
-- answering 404 until somebody writes one.
CREATE TABLE IF NOT EXISTS ad_advertisers (
  id         VARCHAR(24) PRIMARY KEY,
  name       VARCHAR(120) NOT NULL,
  email      VARCHAR(191) NOT NULL DEFAULT '',
  website    VARCHAR(500) NOT NULL DEFAULT '',
  notes      VARCHAR(1000) NOT NULL DEFAULT '',
  created_at VARCHAR(32) NOT NULL,
  updated_at VARCHAR(32) NOT NULL
);

CREATE TABLE IF NOT EXISTS ads (
  id              VARCHAR(24) PRIMARY KEY,
  advertiser_id   VARCHAR(24) NULL,
  name            VARCHAR(120) NOT NULL,
  status          VARCHAR(20) NOT NULL DEFAULT 'draft',
  kind            VARCHAR(20) NOT NULL DEFAULT 'image',
  placement       VARCHAR(20) NOT NULL DEFAULT 'post_bottom',
  paragraph       INT NOT NULL DEFAULT 3,
  slot            VARCHAR(40) NOT NULL DEFAULT '',
  image           VARCHAR(1000) NOT NULL DEFAULT '',
  link            VARCHAR(1000) NOT NULL DEFAULT '',
  alt             VARCHAR(200) NOT NULL DEFAULT '',
  headline        VARCHAR(90) NOT NULL DEFAULT '',
  body            VARCHAR(280) NOT NULL DEFAULT '',
  cta             VARCHAR(30) NOT NULL DEFAULT '',
  -- VARCHAR, not TEXT: sqlx `Any` cannot read TEXT back from MySQL. 6,000
  -- characters is 24,000 bytes of the 65,535-byte row under utf8mb4.
  code            VARCHAR(6000) NOT NULL DEFAULT '',
  audience        VARCHAR(20) NOT NULL DEFAULT 'everyone',
  device          VARCHAR(20) NOT NULL DEFAULT 'all',
  targeting       VARCHAR(20) NOT NULL DEFAULT 'everywhere',
  rules           VARCHAR(2000) NOT NULL DEFAULT '',
  weight          INT NOT NULL DEFAULT 1,
  starts_at       VARCHAR(32) NULL,
  ends_at         VARCHAR(32) NULL,
  max_impressions BIGINT NOT NULL DEFAULT 0,
  max_clicks      BIGINT NOT NULL DEFAULT 0,
  cpm_cents       BIGINT NOT NULL DEFAULT 0,
  cpc_cents       BIGINT NOT NULL DEFAULT 0,
  impressions     BIGINT NOT NULL DEFAULT 0,
  clicks          BIGINT NOT NULL DEFAULT 0,
  created_at      VARCHAR(32) NOT NULL,
  updated_at      VARCHAR(32) NOT NULL,
  INDEX ads_status_idx (status),
  INDEX ads_advertiser_idx (advertiser_id)
);

CREATE TABLE IF NOT EXISTS ad_daily (
  ad_id       VARCHAR(24) NOT NULL,
  day         VARCHAR(10) NOT NULL,
  impressions BIGINT NOT NULL DEFAULT 0,
  clicks      BIGINT NOT NULL DEFAULT 0,
  PRIMARY KEY (ad_id, day),
  INDEX ad_daily_day_idx (day)
);

INSERT IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(RANDOM_BYTES(12))),'ads','ads_enabled','false','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'ads','ads_label','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'ads','ads_hide_paid','true','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'ads','ads_max_in_content','3','number',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'ads','ads_txt','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP());
