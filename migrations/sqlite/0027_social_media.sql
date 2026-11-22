-- ============================================================================
-- 0027_social_media.sql — Tools → Social → Social media
-- ============================================================================
--
-- The accounts this site posts to on other platforms, and every share it has
-- made or will make. Not the social web (ActivityPub), which lives in the
-- federation tables: that makes the site itself an account; this posts to
-- accounts the site holds elsewhere.
--
-- `social_accounts` — one connected account.
--   network      x | bluesky | mastodon | threads | linkedin | facebook |
--                telegram | discord
--   credentials  JSON of what that network needs (tokens, keys, ids). Kept
--                here and NOT in `settings`, because the settings APIs
--                serialize every row and these are secrets. Never sent back
--                to a browser.
--   auto_share   1 = share every newly published post
--   template     this account's own text template; '' uses the site's
--   status       untested | ok | error, from the last test or share
--
-- `social_shares` — one post shared to one account, once.
--   status       scheduled | queued | sending | sent | failed
--   trigger      auto (on publish) | manual (composed by staff)
--   send_at      when a scheduled share is due
--   remote_url   the post on the other platform, when it says where
--
-- Seven settings rows: the default template, campaign tags, hashtags, and
-- the share buttons under posts — which ship OFF.
CREATE TABLE IF NOT EXISTS social_accounts (
  id              TEXT PRIMARY KEY,
  network         TEXT NOT NULL,
  name            TEXT NOT NULL DEFAULT '',
  credentials     TEXT NOT NULL,
  auto_share      INTEGER NOT NULL DEFAULT 0,
  template        TEXT NOT NULL DEFAULT '',
  status          TEXT NOT NULL DEFAULT 'untested',
  last_error      TEXT NOT NULL DEFAULT '',
  last_checked_at TEXT,
  created_at      TEXT NOT NULL,
  updated_at      TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS social_shares (
  id          TEXT PRIMARY KEY,
  account_id  TEXT NOT NULL,
  post_id     TEXT NOT NULL,
  body        TEXT NOT NULL,
  status      TEXT NOT NULL DEFAULT 'queued',
  trigger_kind TEXT NOT NULL DEFAULT 'manual',
  send_at     TEXT,
  sent_at     TEXT,
  remote_id   TEXT NOT NULL DEFAULT '',
  remote_url  TEXT NOT NULL DEFAULT '',
  error       TEXT NOT NULL DEFAULT '',
  attempts    INTEGER NOT NULL DEFAULT 0,
  created_by  TEXT NOT NULL DEFAULT '',
  created_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS social_shares_status_idx ON social_shares (status, send_at);
CREATE INDEX IF NOT EXISTS social_shares_post_idx ON social_shares (post_id);
CREATE INDEX IF NOT EXISTS social_shares_created_idx ON social_shares (created_at);

INSERT OR IGNORE INTO settings (id,`group`,key,value,type,created_at,updated_at) VALUES
(lower(hex(randomblob(12))),'social','social_share_buttons','false','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'social','social_share_networks','x,facebook,linkedin,bluesky,threads,whatsapp,email,copy','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'social','social_share_position','bottom','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'social','social_share_label','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'social','social_default_template','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'social','social_utm','true','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'social','social_hashtags','false','boolean',datetime('now'),datetime('now'));
