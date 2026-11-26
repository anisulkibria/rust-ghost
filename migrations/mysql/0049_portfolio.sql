-- 0049_portfolio.sql — Tools → Portfolio. See the SQLite twin for what each
-- column is for. 

CREATE TABLE IF NOT EXISTS portfolio_categories (
  id          VARCHAR(24) PRIMARY KEY,
  name        VARCHAR(80) NOT NULL,
  slug        VARCHAR(80) NOT NULL UNIQUE,
  description VARCHAR(300) NOT NULL DEFAULT '',
  tint        VARCHAR(20) NOT NULL DEFAULT 'slate',
  sort_order  INT NOT NULL DEFAULT 0,
  created_at  VARCHAR(32) NOT NULL,
  updated_at  VARCHAR(32) NOT NULL
);

CREATE TABLE IF NOT EXISTS portfolio_projects (
  id               VARCHAR(24) PRIMARY KEY,
  slug             VARCHAR(100) NOT NULL UNIQUE,
  title            VARCHAR(200) NOT NULL,
  subtitle         VARCHAR(240) NOT NULL DEFAULT '',
  summary          VARCHAR(400) NOT NULL DEFAULT '',
  body             MEDIUMTEXT NULL,
  body_lexical     MEDIUMTEXT NULL,
  body_html        MEDIUMTEXT NULL,
  cover            VARCHAR(500) NOT NULL DEFAULT '',
  gallery          MEDIUMTEXT NULL,
  video_url        VARCHAR(500) NOT NULL DEFAULT '',
  client_id        VARCHAR(24) NULL,
  client_name      VARCHAR(120) NOT NULL DEFAULT '',
  year             INT NOT NULL DEFAULT 0,
  duration         VARCHAR(60) NOT NULL DEFAULT '',
  role             VARCHAR(160) NOT NULL DEFAULT '',
  industry         VARCHAR(80) NOT NULL DEFAULT '',
  location         VARCHAR(120) NOT NULL DEFAULT '',
  tools            VARCHAR(1500) NOT NULL DEFAULT '',
  live_url         VARCHAR(500) NOT NULL DEFAULT '',
  repo_url         VARCHAR(500) NOT NULL DEFAULT '',
  metrics          MEDIUMTEXT NULL,
  credits          MEDIUMTEXT NULL,
  testimonial_id   VARCHAR(24) NULL,
  accent           VARCHAR(7) NOT NULL DEFAULT '',
  layout           VARCHAR(20) NOT NULL DEFAULT 'case_study',
  featured         SMALLINT NOT NULL DEFAULT 0,
  status           VARCHAR(20) NOT NULL DEFAULT 'draft',
  password_hash    VARCHAR(120) NOT NULL DEFAULT '',
  sort_order       INT NOT NULL DEFAULT 0,
  views            BIGINT NOT NULL DEFAULT 0,
  published_at     VARCHAR(32) NULL,
  seo_title        VARCHAR(70) NOT NULL DEFAULT '',
  seo_description  VARCHAR(320) NOT NULL DEFAULT '',
  created_at       VARCHAR(32) NOT NULL,
  updated_at       VARCHAR(32) NOT NULL,
  INDEX portfolio_projects_status_idx (status, sort_order)
);

CREATE TABLE IF NOT EXISTS portfolio_project_categories (
  project_id   VARCHAR(24) NOT NULL,
  category_id  VARCHAR(24) NOT NULL,
  PRIMARY KEY (project_id, category_id)
);

CREATE TABLE IF NOT EXISTS portfolio_project_services (
  project_id  VARCHAR(24) NOT NULL,
  service_id  VARCHAR(24) NOT NULL,
  PRIMARY KEY (project_id, service_id)
);

CREATE TABLE IF NOT EXISTS portfolio_services (
  id            VARCHAR(24) PRIMARY KEY,
  slug          VARCHAR(100) NOT NULL UNIQUE,
  name          VARCHAR(120) NOT NULL,
  summary       VARCHAR(300) NOT NULL DEFAULT '',
  description   MEDIUMTEXT NULL,
  image         VARCHAR(500) NOT NULL DEFAULT '',
  icon          VARCHAR(20) NOT NULL DEFAULT 'spark',
  price_from    BIGINT NOT NULL DEFAULT 0,
  price_note    VARCHAR(60) NOT NULL DEFAULT '',
  deliverables  MEDIUMTEXT NULL,
  status        VARCHAR(20) NOT NULL DEFAULT 'active',
  featured      SMALLINT NOT NULL DEFAULT 0,
  sort_order    INT NOT NULL DEFAULT 0,
  created_at    VARCHAR(32) NOT NULL,
  updated_at    VARCHAR(32) NOT NULL
);

CREATE TABLE IF NOT EXISTS portfolio_clients (
  id          VARCHAR(24) PRIMARY KEY,
  name        VARCHAR(120) NOT NULL,
  logo        VARCHAR(500) NOT NULL DEFAULT '',
  url         VARCHAR(500) NOT NULL DEFAULT '',
  industry    VARCHAR(80) NOT NULL DEFAULT '',
  featured    SMALLINT NOT NULL DEFAULT 1,
  sort_order  INT NOT NULL DEFAULT 0,
  created_at  VARCHAR(32) NOT NULL,
  updated_at  VARCHAR(32) NOT NULL
);

CREATE TABLE IF NOT EXISTS portfolio_testimonials (
  id          VARCHAR(24) PRIMARY KEY,
  quote       MEDIUMTEXT NOT NULL,
  author      VARCHAR(120) NOT NULL,
  role        VARCHAR(120) NOT NULL DEFAULT '',
  company     VARCHAR(120) NOT NULL DEFAULT '',
  photo       VARCHAR(500) NOT NULL DEFAULT '',
  rating      INT NOT NULL DEFAULT 0,
  project_id  VARCHAR(24) NULL,
  featured    SMALLINT NOT NULL DEFAULT 0,
  sort_order  INT NOT NULL DEFAULT 0,
  created_at  VARCHAR(32) NOT NULL,
  updated_at  VARCHAR(32) NOT NULL
);

CREATE TABLE IF NOT EXISTS portfolio_team (
  id          VARCHAR(24) PRIMARY KEY,
  slug        VARCHAR(100) NOT NULL UNIQUE,
  name        VARCHAR(120) NOT NULL,
  role        VARCHAR(120) NOT NULL DEFAULT '',
  bio         MEDIUMTEXT NULL,
  photo       VARCHAR(500) NOT NULL DEFAULT '',
  email       VARCHAR(191) NOT NULL DEFAULT '',
  links       MEDIUMTEXT NULL,
  status      VARCHAR(20) NOT NULL DEFAULT 'active',
  sort_order  INT NOT NULL DEFAULT 0,
  created_at  VARCHAR(32) NOT NULL,
  updated_at  VARCHAR(32) NOT NULL
);

CREATE TABLE IF NOT EXISTS portfolio_entries (
  id            VARCHAR(24) PRIMARY KEY,
  kind          VARCHAR(20) NOT NULL DEFAULT 'work',
  title         VARCHAR(160) NOT NULL,
  organisation  VARCHAR(160) NOT NULL DEFAULT '',
  location      VARCHAR(120) NOT NULL DEFAULT '',
  url           VARCHAR(500) NOT NULL DEFAULT '',
  date_from     VARCHAR(7) NOT NULL DEFAULT '',
  date_to       VARCHAR(7) NOT NULL DEFAULT '',
  ongoing       SMALLINT NOT NULL DEFAULT 0,
  description   MEDIUMTEXT NULL,
  sort_order    INT NOT NULL DEFAULT 0,
  created_at    VARCHAR(32) NOT NULL,
  updated_at    VARCHAR(32) NOT NULL
);

CREATE TABLE IF NOT EXISTS portfolio_skills (
  id          VARCHAR(24) PRIMARY KEY,
  name        VARCHAR(80) NOT NULL,
  grp         VARCHAR(60) NOT NULL DEFAULT '',
  level       INT NOT NULL DEFAULT 0,
  sort_order  INT NOT NULL DEFAULT 0,
  created_at  VARCHAR(32) NOT NULL
);

CREATE TABLE IF NOT EXISTS portfolio_enquiries (
  id          VARCHAR(24) PRIMARY KEY,
  name        VARCHAR(120) NOT NULL,
  email       VARCHAR(191) NOT NULL,
  company     VARCHAR(120) NOT NULL DEFAULT '',
  phone       VARCHAR(40) NOT NULL DEFAULT '',
  budget      VARCHAR(60) NOT NULL DEFAULT '',
  timeline    VARCHAR(80) NOT NULL DEFAULT '',
  services    VARCHAR(1500) NOT NULL DEFAULT '',
  project     VARCHAR(200) NOT NULL DEFAULT '',
  message     MEDIUMTEXT NULL,
  status      VARCHAR(20) NOT NULL DEFAULT 'new',
  note        MEDIUMTEXT NULL,
  created_at  VARCHAR(32) NOT NULL,
  updated_at  VARCHAR(32) NOT NULL,
  INDEX portfolio_enquiries_status_idx (status, created_at)
);

CREATE TABLE IF NOT EXISTS portfolio_config (
  id      INT PRIMARY KEY,
  secret  VARCHAR(64) NOT NULL
);
INSERT IGNORE INTO portfolio_config (id, secret) VALUES (1, lower(hex(RANDOM_BYTES(32))));

INSERT IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(RANDOM_BYTES(12))),'portfolio','portfolio_enabled','false','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'portfolio','portfolio_base_path','portfolio','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'portfolio','portfolio_mode','personal','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'portfolio','portfolio_title','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'portfolio','portfolio_intro','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'portfolio','portfolio_name','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'portfolio','portfolio_headline','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'portfolio','portfolio_about','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'portfolio','portfolio_portrait','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'portfolio','portfolio_location','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'portfolio','portfolio_availability','hidden','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'portfolio','portfolio_availability_note','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'portfolio','portfolio_email','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'portfolio','portfolio_phone','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'portfolio','portfolio_resume_url','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'portfolio','portfolio_socials','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'portfolio','portfolio_founded','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'portfolio','portfolio_enquiries','true','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'portfolio','portfolio_budgets','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'portfolio','portfolio_notify_email','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'portfolio','portfolio_currency','USD','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'portfolio','portfolio_per_page','24','number',UTC_TIMESTAMP(),UTC_TIMESTAMP());
