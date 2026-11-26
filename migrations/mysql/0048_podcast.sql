-- 0048_podcast.sql — Tools → Podcast. See the SQLite twin for what each
-- column is for.
CREATE TABLE IF NOT EXISTS podcast_shows (
  id               VARCHAR(24) PRIMARY KEY,
  slug             VARCHAR(100) NOT NULL UNIQUE,
  title            VARCHAR(160) NOT NULL,
  subtitle         VARCHAR(255) NOT NULL DEFAULT '',
  description      MEDIUMTEXT NULL,
  artwork          VARCHAR(500) NOT NULL DEFAULT '',
  banner           VARCHAR(500) NOT NULL DEFAULT '',
  author           VARCHAR(160) NOT NULL DEFAULT '',
  owner_name       VARCHAR(160) NOT NULL DEFAULT '',
  owner_email      VARCHAR(191) NOT NULL DEFAULT '',
  category         VARCHAR(80) NOT NULL DEFAULT '',
  category2        VARCHAR(80) NOT NULL DEFAULT '',
  language         VARCHAR(10) NOT NULL DEFAULT 'en',
  explicit         SMALLINT NOT NULL DEFAULT 0,
  show_type        VARCHAR(20) NOT NULL DEFAULT 'episodic',
  copyright        VARCHAR(200) NOT NULL DEFAULT '',
  links            MEDIUMTEXT NULL,
  funding_url      VARCHAR(500) NOT NULL DEFAULT '',
  funding_label    VARCHAR(80) NOT NULL DEFAULT '',
  guid             VARCHAR(80) NOT NULL DEFAULT '',
  locked           SMALLINT NOT NULL DEFAULT 0,
  complete         SMALLINT NOT NULL DEFAULT 0,
  new_feed_url     VARCHAR(500) NOT NULL DEFAULT '',
  tint             VARCHAR(20) NOT NULL DEFAULT 'purple',
  status           VARCHAR(20) NOT NULL DEFAULT 'active',
  sort_order       INT NOT NULL DEFAULT 0,
  seo_title        VARCHAR(70) NOT NULL DEFAULT '',
  seo_description  VARCHAR(320) NOT NULL DEFAULT '',
  created_at       VARCHAR(32) NOT NULL,
  updated_at       VARCHAR(32) NOT NULL
);

CREATE TABLE IF NOT EXISTS podcast_episodes (
  id               VARCHAR(24) PRIMARY KEY,
  show_id          VARCHAR(24) NOT NULL,
  slug             VARCHAR(100) NOT NULL,
  title            VARCHAR(200) NOT NULL,
  summary          VARCHAR(400) NOT NULL DEFAULT '',
  notes            MEDIUMTEXT NULL,
  notes_lexical    MEDIUMTEXT NULL,
  notes_html       MEDIUMTEXT NULL,
  audio_url        VARCHAR(1000) NOT NULL DEFAULT '',
  audio_type       VARCHAR(40) NOT NULL DEFAULT 'audio/mpeg',
  audio_bytes      BIGINT NOT NULL DEFAULT 0,
  duration         INT NOT NULL DEFAULT 0,
  video_url        VARCHAR(500) NOT NULL DEFAULT '',
  artwork          VARCHAR(500) NOT NULL DEFAULT '',
  season           INT NOT NULL DEFAULT 0,
  number           INT NOT NULL DEFAULT 0,
  episode_type     VARCHAR(20) NOT NULL DEFAULT 'full',
  explicit         SMALLINT NOT NULL DEFAULT 0,
  access           VARCHAR(20) NOT NULL DEFAULT 'public',
  status           VARCHAR(20) NOT NULL DEFAULT 'draft',
  published_at     VARCHAR(32) NULL,
  transcript       MEDIUMTEXT NULL,
  chapters         MEDIUMTEXT NULL,
  guid             VARCHAR(300) NOT NULL DEFAULT '',
  featured         SMALLINT NOT NULL DEFAULT 0,
  downloads        BIGINT NOT NULL DEFAULT 0,
  seo_title        VARCHAR(70) NOT NULL DEFAULT '',
  seo_description  VARCHAR(320) NOT NULL DEFAULT '',
  created_at       VARCHAR(32) NOT NULL,
  updated_at       VARCHAR(32) NOT NULL,
  UNIQUE KEY podcast_episodes_slug (show_id, slug),
  INDEX podcast_episodes_show_idx (show_id, status, published_at)
);

CREATE TABLE IF NOT EXISTS podcast_people (
  id          VARCHAR(24) PRIMARY KEY,
  slug        VARCHAR(100) NOT NULL UNIQUE,
  name        VARCHAR(120) NOT NULL,
  title       VARCHAR(160) NOT NULL DEFAULT '',
  bio         MEDIUMTEXT NULL,
  photo       VARCHAR(500) NOT NULL DEFAULT '',
  website     VARCHAR(500) NOT NULL DEFAULT '',
  links       MEDIUMTEXT NULL,
  created_at  VARCHAR(32) NOT NULL,
  updated_at  VARCHAR(32) NOT NULL
);

CREATE TABLE IF NOT EXISTS podcast_credits (
  owner_kind  VARCHAR(10) NOT NULL,
  owner_id    VARCHAR(24) NOT NULL,
  person_id   VARCHAR(24) NOT NULL,
  role        VARCHAR(20) NOT NULL DEFAULT 'guest',
  sort_order  INT NOT NULL DEFAULT 0,
  PRIMARY KEY (owner_kind, owner_id, person_id),
  INDEX podcast_credits_person_idx (person_id)
);

CREATE TABLE IF NOT EXISTS podcast_sponsors (
  id          VARCHAR(24) PRIMARY KEY,
  name        VARCHAR(120) NOT NULL,
  tagline     VARCHAR(240) NOT NULL DEFAULT '',
  url         VARCHAR(500) NOT NULL DEFAULT '',
  logo        VARCHAR(500) NOT NULL DEFAULT '',
  promo_code  VARCHAR(40) NOT NULL DEFAULT '',
  active      SMALLINT NOT NULL DEFAULT 1,
  created_at  VARCHAR(32) NOT NULL,
  updated_at  VARCHAR(32) NOT NULL
);

CREATE TABLE IF NOT EXISTS podcast_episode_sponsors (
  episode_id  VARCHAR(24) NOT NULL,
  sponsor_id  VARCHAR(24) NOT NULL,
  PRIMARY KEY (episode_id, sponsor_id)
);

CREATE TABLE IF NOT EXISTS podcast_downloads (
  id          VARCHAR(24) PRIMARY KEY,
  episode_id  VARCHAR(24) NOT NULL,
  show_id     VARCHAR(24) NOT NULL,
  day         VARCHAR(10) NOT NULL,
  listener    VARCHAR(40) NOT NULL,
  app         VARCHAR(40) NOT NULL DEFAULT 'Other',
  country     VARCHAR(2) NOT NULL DEFAULT '',
  source      VARCHAR(10) NOT NULL DEFAULT 'feed',
  created_at  VARCHAR(32) NOT NULL,
  UNIQUE KEY podcast_downloads_once (episode_id, day, listener),
  INDEX podcast_downloads_day_idx (day, show_id)
);

CREATE TABLE IF NOT EXISTS podcast_requests (
  id          VARCHAR(24) PRIMARY KEY,
  show_id     VARCHAR(24) NULL,
  kind        VARCHAR(20) NOT NULL DEFAULT 'question',
  name        VARCHAR(120) NOT NULL,
  email       VARCHAR(191) NOT NULL,
  topic       VARCHAR(200) NOT NULL DEFAULT '',
  message     MEDIUMTEXT NULL,
  link        VARCHAR(500) NOT NULL DEFAULT '',
  status      VARCHAR(20) NOT NULL DEFAULT 'new',
  note        MEDIUMTEXT NULL,
  created_at  VARCHAR(32) NOT NULL,
  updated_at  VARCHAR(32) NOT NULL,
  INDEX podcast_requests_status_idx (status, created_at)
);

CREATE TABLE IF NOT EXISTS podcast_reviews (
  id          VARCHAR(24) PRIMARY KEY,
  show_id     VARCHAR(24) NOT NULL,
  author      VARCHAR(120) NOT NULL DEFAULT '',
  rating      INT NOT NULL DEFAULT 5,
  title       VARCHAR(160) NOT NULL DEFAULT '',
  body        MEDIUMTEXT NULL,
  source      VARCHAR(20) NOT NULL DEFAULT 'other',
  featured    SMALLINT NOT NULL DEFAULT 0,
  created_at  VARCHAR(32) NOT NULL
);

CREATE TABLE IF NOT EXISTS podcast_config (
  id          INT PRIMARY KEY,
  secret      VARCHAR(64) NOT NULL,
  salt        VARCHAR(32) NOT NULL
);
INSERT IGNORE INTO podcast_config (id, secret, salt) VALUES (1, lower(hex(RANDOM_BYTES(32))), lower(hex(RANDOM_BYTES(16))));

INSERT IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(RANDOM_BYTES(12))),'podcast','podcast_enabled','false','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'podcast','podcast_title','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'podcast','podcast_intro','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'podcast','podcast_base_path','podcast','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'podcast','podcast_count_downloads','true','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'podcast','podcast_per_page','12','number',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'podcast','podcast_notify_email','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'podcast','podcast_requests','true','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP());
