-- ============================================================================
-- 0048_podcast.sql — Tools → Podcast
-- ============================================================================
--
-- What Transistor, Buzzsprout, Captivate, Podbean and Libsyn give a show.
--
-- `podcast_shows` — a show (a site can run several).
--   category, category2   Apple's, "Arts" or "Arts > Design"
--   show_type       episodic | serial
--   links           JSON {platform key: url} (vuuut_core::podcast::PLATFORMS)
--   guid            podcast:guid, made once and never changed
--   locked          podcast:locked — other hosts may not import the feed
--   complete        itunes:complete — no more episodes
--   new_feed_url    itunes:new-feed-url, when the show moves away
--   status          active | hidden (feed and link only) | draft
--
-- `podcast_episodes` — one episode.
--   audio_url       /content/media/… or an https address elsewhere
--   duration        seconds
--   episode_type    full | trailer | bonus
--   access          public | members | paid — gated episodes are left out
--                   of the public feed and are in each member's private feed
--   status          draft | published; published with a future
--                   published_at is scheduled
--   transcript      WebVTT, SRT or plain text, as pasted
--   chapters        JSON [{"start": secs, "title", "url"}]
--   guid            kept from an imported feed so apps do not re-download
--   downloads       running total, for sorting; the detail is below
--
-- `podcast_people` — hosts, co-hosts and guests.
-- `podcast_credits` — who is on a show (owner_kind 'show') or an episode
--   ('episode'), and as what (host | cohost | guest | producer | editor).
-- `podcast_sponsors`, `podcast_episode_sponsors` — read on which episode.
-- `podcast_downloads` — one row per listener per episode per day (IAB-style
--   de-duplication). `listener` is a salted hash, never an address.
-- `podcast_requests` — guest applications and listener questions.
-- `podcast_reviews` — listener reviews the team chose to show.
-- `podcast_config` — one row: the secret that signs private feeds and the
--   salt for listener hashes. Not in `settings`, which the APIs serialise.
--
-- Ships switched off.
CREATE TABLE IF NOT EXISTS podcast_shows (
  id               TEXT PRIMARY KEY,
  slug             TEXT NOT NULL UNIQUE,
  title            TEXT NOT NULL,
  subtitle         TEXT NOT NULL DEFAULT '',
  description      TEXT NOT NULL DEFAULT '',
  artwork          TEXT NOT NULL DEFAULT '',
  banner           TEXT NOT NULL DEFAULT '',
  author           TEXT NOT NULL DEFAULT '',
  owner_name       TEXT NOT NULL DEFAULT '',
  owner_email      TEXT NOT NULL DEFAULT '',
  category         TEXT NOT NULL DEFAULT '',
  category2        TEXT NOT NULL DEFAULT '',
  language         TEXT NOT NULL DEFAULT 'en',
  explicit         INTEGER NOT NULL DEFAULT 0,
  show_type        TEXT NOT NULL DEFAULT 'episodic',
  copyright        TEXT NOT NULL DEFAULT '',
  links            TEXT NOT NULL DEFAULT '',
  funding_url      TEXT NOT NULL DEFAULT '',
  funding_label    TEXT NOT NULL DEFAULT '',
  guid             TEXT NOT NULL DEFAULT '',
  locked           INTEGER NOT NULL DEFAULT 0,
  complete         INTEGER NOT NULL DEFAULT 0,
  new_feed_url     TEXT NOT NULL DEFAULT '',
  tint             TEXT NOT NULL DEFAULT 'purple',
  status           TEXT NOT NULL DEFAULT 'active',
  sort_order       INTEGER NOT NULL DEFAULT 0,
  seo_title        TEXT NOT NULL DEFAULT '',
  seo_description  TEXT NOT NULL DEFAULT '',
  created_at       TEXT NOT NULL,
  updated_at       TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS podcast_episodes (
  id               TEXT PRIMARY KEY,
  show_id          TEXT NOT NULL,
  slug             TEXT NOT NULL,
  title            TEXT NOT NULL,
  summary          TEXT NOT NULL DEFAULT '',
  notes            TEXT NOT NULL DEFAULT '',
  notes_lexical    TEXT NOT NULL DEFAULT '',
  notes_html       TEXT NOT NULL DEFAULT '',
  audio_url        TEXT NOT NULL DEFAULT '',
  audio_type       TEXT NOT NULL DEFAULT 'audio/mpeg',
  audio_bytes      INTEGER NOT NULL DEFAULT 0,
  duration         INTEGER NOT NULL DEFAULT 0,
  video_url        TEXT NOT NULL DEFAULT '',
  artwork          TEXT NOT NULL DEFAULT '',
  season           INTEGER NOT NULL DEFAULT 0,
  number           INTEGER NOT NULL DEFAULT 0,
  episode_type     TEXT NOT NULL DEFAULT 'full',
  explicit         INTEGER NOT NULL DEFAULT 0,
  access           TEXT NOT NULL DEFAULT 'public',
  status           TEXT NOT NULL DEFAULT 'draft',
  published_at     TEXT,
  transcript       TEXT NOT NULL DEFAULT '',
  chapters         TEXT NOT NULL DEFAULT '',
  guid             TEXT NOT NULL DEFAULT '',
  featured         INTEGER NOT NULL DEFAULT 0,
  downloads        INTEGER NOT NULL DEFAULT 0,
  seo_title        TEXT NOT NULL DEFAULT '',
  seo_description  TEXT NOT NULL DEFAULT '',
  created_at       TEXT NOT NULL,
  updated_at       TEXT NOT NULL,
  UNIQUE (show_id, slug)
);
CREATE INDEX IF NOT EXISTS podcast_episodes_show_idx ON podcast_episodes (show_id, status, published_at);

CREATE TABLE IF NOT EXISTS podcast_people (
  id          TEXT PRIMARY KEY,
  slug        TEXT NOT NULL UNIQUE,
  name        TEXT NOT NULL,
  title       TEXT NOT NULL DEFAULT '',
  bio         TEXT NOT NULL DEFAULT '',
  photo       TEXT NOT NULL DEFAULT '',
  website     TEXT NOT NULL DEFAULT '',
  links       TEXT NOT NULL DEFAULT '',
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS podcast_credits (
  owner_kind  TEXT NOT NULL,
  owner_id    TEXT NOT NULL,
  person_id   TEXT NOT NULL,
  role        TEXT NOT NULL DEFAULT 'guest',
  sort_order  INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (owner_kind, owner_id, person_id)
);
CREATE INDEX IF NOT EXISTS podcast_credits_person_idx ON podcast_credits (person_id);

CREATE TABLE IF NOT EXISTS podcast_sponsors (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  tagline     TEXT NOT NULL DEFAULT '',
  url         TEXT NOT NULL DEFAULT '',
  logo        TEXT NOT NULL DEFAULT '',
  promo_code  TEXT NOT NULL DEFAULT '',
  active      INTEGER NOT NULL DEFAULT 1,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS podcast_episode_sponsors (
  episode_id  TEXT NOT NULL,
  sponsor_id  TEXT NOT NULL,
  PRIMARY KEY (episode_id, sponsor_id)
);

CREATE TABLE IF NOT EXISTS podcast_downloads (
  id          TEXT PRIMARY KEY,
  episode_id  TEXT NOT NULL,
  show_id     TEXT NOT NULL,
  day         TEXT NOT NULL,
  listener    TEXT NOT NULL,
  app         TEXT NOT NULL DEFAULT 'Other',
  country     TEXT NOT NULL DEFAULT '',
  source      TEXT NOT NULL DEFAULT 'feed',
  created_at  TEXT NOT NULL,
  UNIQUE (episode_id, day, listener)
);
CREATE INDEX IF NOT EXISTS podcast_downloads_day_idx ON podcast_downloads (day, show_id);

CREATE TABLE IF NOT EXISTS podcast_requests (
  id          TEXT PRIMARY KEY,
  show_id     TEXT,
  kind        TEXT NOT NULL DEFAULT 'question',
  name        TEXT NOT NULL,
  email       TEXT NOT NULL,
  topic       TEXT NOT NULL DEFAULT '',
  message     TEXT NOT NULL DEFAULT '',
  link        TEXT NOT NULL DEFAULT '',
  status      TEXT NOT NULL DEFAULT 'new',
  note        TEXT NOT NULL DEFAULT '',
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS podcast_requests_status_idx ON podcast_requests (status, created_at);

CREATE TABLE IF NOT EXISTS podcast_reviews (
  id          TEXT PRIMARY KEY,
  show_id     TEXT NOT NULL,
  author      TEXT NOT NULL DEFAULT '',
  rating      INTEGER NOT NULL DEFAULT 5,
  title       TEXT NOT NULL DEFAULT '',
  body        TEXT NOT NULL DEFAULT '',
  source      TEXT NOT NULL DEFAULT 'other',
  featured    INTEGER NOT NULL DEFAULT 0,
  created_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS podcast_config (
  id          INTEGER PRIMARY KEY,
  secret      TEXT NOT NULL,
  salt        TEXT NOT NULL
);
INSERT OR IGNORE INTO podcast_config (id, secret, salt) VALUES (1, lower(hex(randomblob(32))), lower(hex(randomblob(16))));

INSERT OR IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(randomblob(12))),'podcast','podcast_enabled','false','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'podcast','podcast_title','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'podcast','podcast_intro','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'podcast','podcast_base_path','podcast','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'podcast','podcast_count_downloads','true','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'podcast','podcast_per_page','12','number',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'podcast','podcast_notify_email','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'podcast','podcast_requests','true','boolean',datetime('now'),datetime('now'));
