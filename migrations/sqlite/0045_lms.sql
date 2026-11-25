-- ============================================================================
-- 0045_lms.sql — Tools → LMS & Online Courses
-- ============================================================================
--
-- Courses made of sections and lessons, sold or given to members, with
-- progress, quizzes, assignments, reviews and certificates. What LearnDash,
-- LifterLMS, Tutor LMS, Sensei and LearnPress have in common.
--
-- `lms_categories` — how courses are grouped in the catalogue.
--
-- `lms_courses` — one course.
--   status        draft | published | archived
--   access        open      anyone may read every lesson, no account needed
--                 free      free, but a member account enrols you
--                 members   any signed-in member; paid members only when
--                           `tier_ids` is `paid`
--                 tiers     members of one of `tier_ids` (comma list)
--                 paid      bought through the store: `product_id` is the
--                           e-commerce product that grants it when paid
--   description*  the editor's Lexical and HTML, and a plain copy for search
--   outcomes, requirements, audience  one item per line
--   sequential    lessons unlock in order
--   capacity      0 = unlimited seats
--   access_days   0 = lifetime access after enrolment
--   opens_at / closes_at  the enrolment window, or NULL
--
-- `lms_sections` — the course's chapters (LearnDash "sections", Tutor
-- "topics"), ordered.
--
-- `lms_lessons` — a lesson, quiz or assignment, in a section.
--   kind          lesson | quiz | assignment
--   preview       readable before enrolling
--   drip_days     unlocks this many days after enrolment (0 = at once)
--   attachments   one `label|address` per line
--   pass_mark, attempts (0 = unlimited), time_limit (minutes), shuffle,
--   reveal (show the right answers after a try) — quizzes
--   points        an assignment's maximum grade
--
-- `lms_questions` — a quiz's questions.
--   kind          single | multiple | truefalse | text
--   options       JSON [{"t": text, "c": correct}]
--   answer        accepted answers to a text question, `|`-separated
--
-- `lms_enrollments` — a member in a course. One row per pair.
--   status        active | completed | expired | cancelled
--   source        free | staff | purchase | tier
--   progress      0–100, recomputed whenever a lesson is completed
--   certificate_code  set on completion when certificates are on; it is the
--                 public verification address
--
-- `lms_progress` — a completed lesson. `lms_attempts` — a quiz try.
-- `lms_submissions` — an assignment handed in; staff grade it.
-- `lms_reviews` — a student's rating and review, one per course.
--
-- Ships switched off: no public catalogue until somebody turns it on.
CREATE TABLE IF NOT EXISTS lms_categories (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  slug        TEXT NOT NULL UNIQUE,
  description TEXT NOT NULL DEFAULT '',
  tint        TEXT NOT NULL DEFAULT 'blue',
  image       TEXT NOT NULL DEFAULT '',
  sort_order  INTEGER NOT NULL DEFAULT 0,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS lms_courses (
  id                  TEXT PRIMARY KEY,
  slug                TEXT NOT NULL UNIQUE,
  title               TEXT NOT NULL,
  subtitle            TEXT NOT NULL DEFAULT '',
  description         TEXT NOT NULL DEFAULT '',
  description_lexical TEXT NOT NULL DEFAULT '',
  description_html    TEXT NOT NULL DEFAULT '',
  image               TEXT NOT NULL DEFAULT '',
  intro_video         TEXT NOT NULL DEFAULT '',
  category_id         TEXT,
  instructor_id       TEXT,
  level               TEXT NOT NULL DEFAULT 'all',
  language            TEXT NOT NULL DEFAULT '',
  duration_minutes    INTEGER NOT NULL DEFAULT 0,
  status              TEXT NOT NULL DEFAULT 'draft',
  access              TEXT NOT NULL DEFAULT 'free',
  price_cents         INTEGER NOT NULL DEFAULT 0,
  compare_cents       INTEGER NOT NULL DEFAULT 0,
  product_id          TEXT,
  tier_ids            TEXT NOT NULL DEFAULT '',
  outcomes            TEXT NOT NULL DEFAULT '',
  requirements        TEXT NOT NULL DEFAULT '',
  audience            TEXT NOT NULL DEFAULT '',
  keywords            TEXT NOT NULL DEFAULT '',
  featured            INTEGER NOT NULL DEFAULT 0,
  sequential          INTEGER NOT NULL DEFAULT 0,
  certificate         INTEGER NOT NULL DEFAULT 1,
  capacity            INTEGER NOT NULL DEFAULT 0,
  access_days         INTEGER NOT NULL DEFAULT 0,
  opens_at            TEXT,
  closes_at           TEXT,
  seo_title           TEXT NOT NULL DEFAULT '',
  seo_description     TEXT NOT NULL DEFAULT '',
  views               INTEGER NOT NULL DEFAULT 0,
  sort_order          INTEGER NOT NULL DEFAULT 0,
  published_at        TEXT,
  created_at          TEXT NOT NULL,
  updated_at          TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS lms_courses_status_idx ON lms_courses (status, published_at);
CREATE INDEX IF NOT EXISTS lms_courses_category_idx ON lms_courses (category_id);
CREATE INDEX IF NOT EXISTS lms_courses_product_idx ON lms_courses (product_id);

CREATE TABLE IF NOT EXISTS lms_sections (
  id          TEXT PRIMARY KEY,
  course_id   TEXT NOT NULL,
  title       TEXT NOT NULL,
  summary     TEXT NOT NULL DEFAULT '',
  sort_order  INTEGER NOT NULL DEFAULT 0,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS lms_sections_course_idx ON lms_sections (course_id, sort_order);

CREATE TABLE IF NOT EXISTS lms_lessons (
  id               TEXT PRIMARY KEY,
  course_id        TEXT NOT NULL,
  section_id       TEXT,
  slug             TEXT NOT NULL,
  title            TEXT NOT NULL,
  kind             TEXT NOT NULL DEFAULT 'lesson',
  status           TEXT NOT NULL DEFAULT 'published',
  summary          TEXT NOT NULL DEFAULT '',
  content_lexical  TEXT NOT NULL DEFAULT '',
  content_html     TEXT NOT NULL DEFAULT '',
  video_url        TEXT NOT NULL DEFAULT '',
  duration_minutes INTEGER NOT NULL DEFAULT 0,
  preview          INTEGER NOT NULL DEFAULT 0,
  drip_days        INTEGER NOT NULL DEFAULT 0,
  attachments      TEXT NOT NULL DEFAULT '',
  pass_mark        INTEGER NOT NULL DEFAULT 70,
  attempts         INTEGER NOT NULL DEFAULT 0,
  time_limit       INTEGER NOT NULL DEFAULT 0,
  shuffle          INTEGER NOT NULL DEFAULT 0,
  reveal           INTEGER NOT NULL DEFAULT 1,
  points           INTEGER NOT NULL DEFAULT 100,
  sort_order       INTEGER NOT NULL DEFAULT 0,
  created_at       TEXT NOT NULL,
  updated_at       TEXT NOT NULL,
  UNIQUE (course_id, slug)
);
CREATE INDEX IF NOT EXISTS lms_lessons_section_idx ON lms_lessons (section_id, sort_order);

CREATE TABLE IF NOT EXISTS lms_questions (
  id          TEXT PRIMARY KEY,
  lesson_id   TEXT NOT NULL,
  kind        TEXT NOT NULL DEFAULT 'single',
  prompt      TEXT NOT NULL,
  options     TEXT NOT NULL DEFAULT '',
  answer      TEXT NOT NULL DEFAULT '',
  explanation TEXT NOT NULL DEFAULT '',
  points      INTEGER NOT NULL DEFAULT 1,
  sort_order  INTEGER NOT NULL DEFAULT 0,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS lms_questions_lesson_idx ON lms_questions (lesson_id, sort_order);

CREATE TABLE IF NOT EXISTS lms_enrollments (
  id               TEXT PRIMARY KEY,
  course_id        TEXT NOT NULL,
  member_id        TEXT NOT NULL,
  status           TEXT NOT NULL DEFAULT 'active',
  source           TEXT NOT NULL DEFAULT 'free',
  order_id         TEXT,
  progress         INTEGER NOT NULL DEFAULT 0,
  last_lesson_id   TEXT,
  certificate_code TEXT,
  enrolled_at      TEXT NOT NULL,
  completed_at     TEXT,
  expires_at       TEXT,
  last_active_at   TEXT,
  created_at       TEXT NOT NULL,
  updated_at       TEXT NOT NULL,
  UNIQUE (course_id, member_id)
);
CREATE INDEX IF NOT EXISTS lms_enrollments_member_idx ON lms_enrollments (member_id);
CREATE INDEX IF NOT EXISTS lms_enrollments_cert_idx ON lms_enrollments (certificate_code);

CREATE TABLE IF NOT EXISTS lms_progress (
  id            TEXT PRIMARY KEY,
  enrollment_id TEXT NOT NULL,
  lesson_id     TEXT NOT NULL,
  completed_at  TEXT NOT NULL,
  UNIQUE (enrollment_id, lesson_id)
);

CREATE TABLE IF NOT EXISTS lms_attempts (
  id            TEXT PRIMARY KEY,
  enrollment_id TEXT NOT NULL,
  lesson_id     TEXT NOT NULL,
  score         INTEGER NOT NULL DEFAULT 0,
  points        INTEGER NOT NULL DEFAULT 0,
  max_points    INTEGER NOT NULL DEFAULT 0,
  passed        INTEGER NOT NULL DEFAULT 0,
  answers       TEXT NOT NULL DEFAULT '',
  started_at    TEXT,
  submitted_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS lms_attempts_idx ON lms_attempts (enrollment_id, lesson_id);

CREATE TABLE IF NOT EXISTS lms_submissions (
  id            TEXT PRIMARY KEY,
  enrollment_id TEXT NOT NULL,
  lesson_id     TEXT NOT NULL,
  body          TEXT NOT NULL DEFAULT '',
  link          TEXT NOT NULL DEFAULT '',
  status        TEXT NOT NULL DEFAULT 'submitted',
  grade         INTEGER NOT NULL DEFAULT 0,
  feedback      TEXT NOT NULL DEFAULT '',
  submitted_at  TEXT NOT NULL,
  reviewed_at   TEXT,
  reviewed_by   TEXT,
  UNIQUE (enrollment_id, lesson_id)
);
CREATE INDEX IF NOT EXISTS lms_submissions_status_idx ON lms_submissions (status, submitted_at);

CREATE TABLE IF NOT EXISTS lms_reviews (
  id          TEXT PRIMARY KEY,
  course_id   TEXT NOT NULL,
  member_id   TEXT NOT NULL,
  rating      INTEGER NOT NULL DEFAULT 5,
  title       TEXT NOT NULL DEFAULT '',
  body        TEXT NOT NULL DEFAULT '',
  status      TEXT NOT NULL DEFAULT 'published',
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL,
  UNIQUE (course_id, member_id)
);
CREATE INDEX IF NOT EXISTS lms_reviews_course_idx ON lms_reviews (course_id, status);

INSERT OR IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(randomblob(12))),'lms','lms_enabled','false','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'lms','lms_title','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'lms','lms_intro','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'lms','lms_base_path','courses','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'lms','lms_per_page','12','number',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'lms','lms_currency','USD','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'lms','lms_reviews','true','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'lms','lms_moderate_reviews','false','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'lms','lms_certificates','true','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'lms','lms_certificate_signer','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'lms','lms_show_students','true','boolean',datetime('now'),datetime('now'));
