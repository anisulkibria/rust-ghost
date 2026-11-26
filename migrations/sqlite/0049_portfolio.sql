-- ============================================================================
-- 0049_portfolio.sql — Tools → Portfolio
-- ============================================================================
--
-- A personal portfolio (projects, a résumé, skills, "hire me") and a studio
-- portfolio (projects, services, clients, team, a project brief) share these.
--
-- `portfolio_categories` — disciplines the grid filters by (Branding, Web…).
--
-- `portfolio_projects` — one project or case study.
--   cover, gallery  the first picture and the rest (newline list)
--   client_id       a row in portfolio_clients, or NULL and client_name
--   tools           comma list (Figma, React, Blender…)
--   metrics         JSON [{"value": "+38%", "label": "conversion"}]
--   credits         JSON [{"name", "role", "url"}]
--   accent          #hex the page is tinted with, or empty
--   layout          case_study | gallery | minimal
--   status          published | protected (listed, opens with a password)
--                   | unlisted (by link) | draft
--   password_hash   salt$sha256 (vuuut_core::portfolio::hash_password)
--   views           counted on page-cache misses, like the directory
--
-- `portfolio_project_categories`, `portfolio_project_services` — links.
-- `portfolio_services` — what can be hired: price_from in cents, 0 = no
--   price shown; deliverables a newline list.
-- `portfolio_clients` — names and logos; featured = on the logo wall.
-- `portfolio_testimonials` — quotes, optionally tied to a project.
-- `portfolio_team` — the people of a studio (or a person's collaborators).
-- `portfolio_entries` — the résumé: kind work | education | award | press |
--   talk | publication | certification | volunteer; date_from/date_to YYYY or
--   YYYY-MM; ongoing = to the present.
-- `portfolio_skills` — name, group, level 0–100 (0 = no bar).
-- `portfolio_enquiries` — the contact / project-brief form. status new |
--   replied | proposal | won | lost | archived | spam.
-- `portfolio_config` — one row: the secret that signs unlock cookies.
--
-- Ships switched off.
CREATE TABLE IF NOT EXISTS portfolio_categories (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  slug        TEXT NOT NULL UNIQUE,
  description TEXT NOT NULL DEFAULT '',
  tint        TEXT NOT NULL DEFAULT 'slate',
  sort_order  INTEGER NOT NULL DEFAULT 0,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS portfolio_projects (
  id               TEXT PRIMARY KEY,
  slug             TEXT NOT NULL UNIQUE,
  title            TEXT NOT NULL,
  subtitle         TEXT NOT NULL DEFAULT '',
  summary          TEXT NOT NULL DEFAULT '',
  body             TEXT NOT NULL DEFAULT '',
  body_lexical     TEXT NOT NULL DEFAULT '',
  body_html        TEXT NOT NULL DEFAULT '',
  cover            TEXT NOT NULL DEFAULT '',
  gallery          TEXT NOT NULL DEFAULT '',
  video_url        TEXT NOT NULL DEFAULT '',
  client_id        TEXT,
  client_name      TEXT NOT NULL DEFAULT '',
  year             INTEGER NOT NULL DEFAULT 0,
  duration         TEXT NOT NULL DEFAULT '',
  role             TEXT NOT NULL DEFAULT '',
  industry         TEXT NOT NULL DEFAULT '',
  location         TEXT NOT NULL DEFAULT '',
  tools            TEXT NOT NULL DEFAULT '',
  live_url         TEXT NOT NULL DEFAULT '',
  repo_url         TEXT NOT NULL DEFAULT '',
  metrics          TEXT NOT NULL DEFAULT '',
  credits          TEXT NOT NULL DEFAULT '',
  testimonial_id   TEXT,
  accent           TEXT NOT NULL DEFAULT '',
  layout           TEXT NOT NULL DEFAULT 'case_study',
  featured         INTEGER NOT NULL DEFAULT 0,
  status           TEXT NOT NULL DEFAULT 'draft',
  password_hash    TEXT NOT NULL DEFAULT '',
  sort_order       INTEGER NOT NULL DEFAULT 0,
  views            INTEGER NOT NULL DEFAULT 0,
  published_at     TEXT,
  seo_title        TEXT NOT NULL DEFAULT '',
  seo_description  TEXT NOT NULL DEFAULT '',
  created_at       TEXT NOT NULL,
  updated_at       TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS portfolio_projects_status_idx ON portfolio_projects (status, sort_order);

CREATE TABLE IF NOT EXISTS portfolio_project_categories (
  project_id   TEXT NOT NULL,
  category_id  TEXT NOT NULL,
  PRIMARY KEY (project_id, category_id)
);

CREATE TABLE IF NOT EXISTS portfolio_project_services (
  project_id  TEXT NOT NULL,
  service_id  TEXT NOT NULL,
  PRIMARY KEY (project_id, service_id)
);

CREATE TABLE IF NOT EXISTS portfolio_services (
  id            TEXT PRIMARY KEY,
  slug          TEXT NOT NULL UNIQUE,
  name          TEXT NOT NULL,
  summary       TEXT NOT NULL DEFAULT '',
  description   TEXT NOT NULL DEFAULT '',
  image         TEXT NOT NULL DEFAULT '',
  icon          TEXT NOT NULL DEFAULT 'spark',
  price_from    INTEGER NOT NULL DEFAULT 0,
  price_note    TEXT NOT NULL DEFAULT '',
  deliverables  TEXT NOT NULL DEFAULT '',
  status        TEXT NOT NULL DEFAULT 'active',
  featured      INTEGER NOT NULL DEFAULT 0,
  sort_order    INTEGER NOT NULL DEFAULT 0,
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS portfolio_clients (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  logo        TEXT NOT NULL DEFAULT '',
  url         TEXT NOT NULL DEFAULT '',
  industry    TEXT NOT NULL DEFAULT '',
  featured    INTEGER NOT NULL DEFAULT 1,
  sort_order  INTEGER NOT NULL DEFAULT 0,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS portfolio_testimonials (
  id          TEXT PRIMARY KEY,
  quote       TEXT NOT NULL,
  author      TEXT NOT NULL,
  role        TEXT NOT NULL DEFAULT '',
  company     TEXT NOT NULL DEFAULT '',
  photo       TEXT NOT NULL DEFAULT '',
  rating      INTEGER NOT NULL DEFAULT 0,
  project_id  TEXT,
  featured    INTEGER NOT NULL DEFAULT 0,
  sort_order  INTEGER NOT NULL DEFAULT 0,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS portfolio_team (
  id          TEXT PRIMARY KEY,
  slug        TEXT NOT NULL UNIQUE,
  name        TEXT NOT NULL,
  role        TEXT NOT NULL DEFAULT '',
  bio         TEXT NOT NULL DEFAULT '',
  photo       TEXT NOT NULL DEFAULT '',
  email       TEXT NOT NULL DEFAULT '',
  links       TEXT NOT NULL DEFAULT '',
  status      TEXT NOT NULL DEFAULT 'active',
  sort_order  INTEGER NOT NULL DEFAULT 0,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS portfolio_entries (
  id            TEXT PRIMARY KEY,
  kind          TEXT NOT NULL DEFAULT 'work',
  title         TEXT NOT NULL,
  organisation  TEXT NOT NULL DEFAULT '',
  location      TEXT NOT NULL DEFAULT '',
  url           TEXT NOT NULL DEFAULT '',
  date_from     TEXT NOT NULL DEFAULT '',
  date_to       TEXT NOT NULL DEFAULT '',
  ongoing       INTEGER NOT NULL DEFAULT 0,
  description   TEXT NOT NULL DEFAULT '',
  sort_order    INTEGER NOT NULL DEFAULT 0,
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS portfolio_skills (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  grp         TEXT NOT NULL DEFAULT '',
  level       INTEGER NOT NULL DEFAULT 0,
  sort_order  INTEGER NOT NULL DEFAULT 0,
  created_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS portfolio_enquiries (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  email       TEXT NOT NULL,
  company     TEXT NOT NULL DEFAULT '',
  phone       TEXT NOT NULL DEFAULT '',
  budget      TEXT NOT NULL DEFAULT '',
  timeline    TEXT NOT NULL DEFAULT '',
  services    TEXT NOT NULL DEFAULT '',
  project     TEXT NOT NULL DEFAULT '',
  message     TEXT NOT NULL DEFAULT '',
  status      TEXT NOT NULL DEFAULT 'new',
  note        TEXT NOT NULL DEFAULT '',
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS portfolio_enquiries_status_idx ON portfolio_enquiries (status, created_at);

CREATE TABLE IF NOT EXISTS portfolio_config (
  id      INTEGER PRIMARY KEY,
  secret  TEXT NOT NULL
);
INSERT OR IGNORE INTO portfolio_config (id, secret) VALUES (1, lower(hex(randomblob(32))));

INSERT OR IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(randomblob(12))),'portfolio','portfolio_enabled','false','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'portfolio','portfolio_base_path','portfolio','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'portfolio','portfolio_mode','personal','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'portfolio','portfolio_title','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'portfolio','portfolio_intro','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'portfolio','portfolio_name','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'portfolio','portfolio_headline','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'portfolio','portfolio_about','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'portfolio','portfolio_portrait','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'portfolio','portfolio_location','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'portfolio','portfolio_availability','hidden','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'portfolio','portfolio_availability_note','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'portfolio','portfolio_email','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'portfolio','portfolio_phone','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'portfolio','portfolio_resume_url','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'portfolio','portfolio_socials','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'portfolio','portfolio_founded','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'portfolio','portfolio_enquiries','true','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'portfolio','portfolio_budgets','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'portfolio','portfolio_notify_email','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'portfolio','portfolio_currency','USD','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'portfolio','portfolio_per_page','24','number',datetime('now'),datetime('now'));
