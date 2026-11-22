-- ============================================================================
-- 0025_ads.sql — Tools → Advertising & Ad Management
-- ============================================================================
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
  id         TEXT PRIMARY KEY,
  name       TEXT NOT NULL,
  email      TEXT NOT NULL DEFAULT '',
  website    TEXT NOT NULL DEFAULT '',
  notes      TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS ads (
  id              TEXT PRIMARY KEY,
  advertiser_id   TEXT,
  name            TEXT NOT NULL,
  status          TEXT NOT NULL DEFAULT 'draft',
  kind            TEXT NOT NULL DEFAULT 'image',
  placement       TEXT NOT NULL DEFAULT 'post_bottom',
  paragraph       INTEGER NOT NULL DEFAULT 3,
  slot            TEXT NOT NULL DEFAULT '',
  image           TEXT NOT NULL DEFAULT '',
  link            TEXT NOT NULL DEFAULT '',
  alt             TEXT NOT NULL DEFAULT '',
  headline        TEXT NOT NULL DEFAULT '',
  body            TEXT NOT NULL DEFAULT '',
  cta             TEXT NOT NULL DEFAULT '',
  code            TEXT NOT NULL DEFAULT '',
  audience        TEXT NOT NULL DEFAULT 'everyone',
  device          TEXT NOT NULL DEFAULT 'all',
  targeting       TEXT NOT NULL DEFAULT 'everywhere',
  rules           TEXT NOT NULL DEFAULT '',
  weight          INTEGER NOT NULL DEFAULT 1,
  starts_at       TEXT,
  ends_at         TEXT,
  max_impressions INTEGER NOT NULL DEFAULT 0,
  max_clicks      INTEGER NOT NULL DEFAULT 0,
  cpm_cents       INTEGER NOT NULL DEFAULT 0,
  cpc_cents       INTEGER NOT NULL DEFAULT 0,
  impressions     INTEGER NOT NULL DEFAULT 0,
  clicks          INTEGER NOT NULL DEFAULT 0,
  created_at      TEXT NOT NULL,
  updated_at      TEXT NOT NULL
);
-- The renderer asks one question per page: which ads are live.
CREATE INDEX IF NOT EXISTS ads_status_idx ON ads (status);
CREATE INDEX IF NOT EXISTS ads_advertiser_idx ON ads (advertiser_id);

CREATE TABLE IF NOT EXISTS ad_daily (
  ad_id       TEXT NOT NULL,
  day         TEXT NOT NULL,
  impressions INTEGER NOT NULL DEFAULT 0,
  clicks      INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (ad_id, day)
);
CREATE INDEX IF NOT EXISTS ad_daily_day_idx ON ad_daily (day);

INSERT OR IGNORE INTO settings (id,`group`,key,value,type,created_at,updated_at) VALUES
(lower(hex(randomblob(12))),'ads','ads_enabled','false','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'ads','ads_label','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'ads','ads_hide_paid','true','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'ads','ads_max_in_content','3','number',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'ads','ads_txt','','string',datetime('now'),datetime('now'));
