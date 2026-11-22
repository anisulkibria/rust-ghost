-- ============================================================================
-- 0027_social_media.sql — Tools → Social → Social media
-- ============================================================================
--
-- MySQL twin of migrations/sqlite/0027_social_media.sql.
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
  id              VARCHAR(24) PRIMARY KEY,
  network         VARCHAR(20) NOT NULL,
  name            VARCHAR(191) NOT NULL DEFAULT '',
  credentials     VARCHAR(4000) NOT NULL,
  auto_share      SMALLINT NOT NULL DEFAULT 0,
  template        VARCHAR(1000) NOT NULL DEFAULT '',
  status          VARCHAR(20) NOT NULL DEFAULT 'untested',
  last_error      VARCHAR(500) NOT NULL DEFAULT '',
  last_checked_at VARCHAR(32) NULL,
  created_at      VARCHAR(32) NOT NULL,
  updated_at      VARCHAR(32) NOT NULL
);

CREATE TABLE IF NOT EXISTS social_shares (
  id           VARCHAR(24) PRIMARY KEY,
  account_id   VARCHAR(24) NOT NULL,
  post_id      VARCHAR(24) NOT NULL,
  body         VARCHAR(5000) NOT NULL,
  status       VARCHAR(20) NOT NULL DEFAULT 'queued',
  trigger_kind VARCHAR(20) NOT NULL DEFAULT 'manual',
  send_at      VARCHAR(32) NULL,
  sent_at      VARCHAR(32) NULL,
  remote_id    VARCHAR(191) NOT NULL DEFAULT '',
  remote_url   VARCHAR(500) NOT NULL DEFAULT '',
  error        VARCHAR(500) NOT NULL DEFAULT '',
  attempts     INT NOT NULL DEFAULT 0,
  created_by   VARCHAR(24) NOT NULL DEFAULT '',
  created_at   VARCHAR(32) NOT NULL,
  INDEX social_shares_status_idx (status, send_at),
  INDEX social_shares_post_idx (post_id),
  INDEX social_shares_created_idx (created_at)
);

INSERT IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(RANDOM_BYTES(12))),'social','social_share_buttons','false','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'social','social_share_networks','x,facebook,linkedin,bluesky,threads,whatsapp,email,copy','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'social','social_share_position','bottom','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'social','social_share_label','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'social','social_default_template','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'social','social_utm','true','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'social','social_hashtags','false','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP());
