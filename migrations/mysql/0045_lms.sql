-- ============================================================================
-- 0045_lms.sql — Tools → LMS & Online Courses
-- ============================================================================
--
-- MySQL twin of migrations/sqlite/0045_lms.sql (see there for every column).
-- Long text — the editor's Lexical and HTML, line lists, quiz options and
-- answers, assignment bodies — is MEDIUMTEXT, nullable because MEDIUMTEXT
-- cannot take a DEFAULT; the repo reads those columns through COALESCE.
CREATE TABLE IF NOT EXISTS lms_categories (
  id          VARCHAR(24) PRIMARY KEY,
  name        VARCHAR(80) NOT NULL,
  slug        VARCHAR(80) NOT NULL UNIQUE,
  description VARCHAR(300) NOT NULL DEFAULT '',
  tint        VARCHAR(20) NOT NULL DEFAULT 'blue',
  image       VARCHAR(500) NOT NULL DEFAULT '',
  sort_order  INT NOT NULL DEFAULT 0,
  created_at  VARCHAR(32) NOT NULL,
  updated_at  VARCHAR(32) NOT NULL
);

CREATE TABLE IF NOT EXISTS lms_courses (
  id                  VARCHAR(24) PRIMARY KEY,
  slug                VARCHAR(100) NOT NULL UNIQUE,
  title               VARCHAR(160) NOT NULL,
  subtitle            VARCHAR(240) NOT NULL DEFAULT '',
  description         MEDIUMTEXT NULL,
  description_lexical MEDIUMTEXT NULL,
  description_html    MEDIUMTEXT NULL,
  image               VARCHAR(500) NOT NULL DEFAULT '',
  intro_video         VARCHAR(500) NOT NULL DEFAULT '',
  category_id         VARCHAR(24) NULL,
  instructor_id       VARCHAR(24) NULL,
  level               VARCHAR(20) NOT NULL DEFAULT 'all',
  language            VARCHAR(40) NOT NULL DEFAULT '',
  duration_minutes    INT NOT NULL DEFAULT 0,
  status              VARCHAR(20) NOT NULL DEFAULT 'draft',
  access              VARCHAR(20) NOT NULL DEFAULT 'free',
  price_cents         BIGINT NOT NULL DEFAULT 0,
  compare_cents       BIGINT NOT NULL DEFAULT 0,
  product_id          VARCHAR(24) NULL,
  tier_ids            VARCHAR(500) NOT NULL DEFAULT '',
  outcomes            MEDIUMTEXT NULL,
  requirements        MEDIUMTEXT NULL,
  audience            MEDIUMTEXT NULL,
  keywords            VARCHAR(200) NOT NULL DEFAULT '',
  featured            SMALLINT NOT NULL DEFAULT 0,
  sequential          SMALLINT NOT NULL DEFAULT 0,
  certificate         SMALLINT NOT NULL DEFAULT 1,
  capacity            INT NOT NULL DEFAULT 0,
  access_days         INT NOT NULL DEFAULT 0,
  opens_at            VARCHAR(32) NULL,
  closes_at           VARCHAR(32) NULL,
  seo_title           VARCHAR(70) NOT NULL DEFAULT '',
  seo_description     VARCHAR(320) NOT NULL DEFAULT '',
  views               BIGINT NOT NULL DEFAULT 0,
  sort_order          INT NOT NULL DEFAULT 0,
  published_at        VARCHAR(32) NULL,
  created_at          VARCHAR(32) NOT NULL,
  updated_at          VARCHAR(32) NOT NULL,
  INDEX lms_courses_status_idx (status, published_at),
  INDEX lms_courses_category_idx (category_id),
  INDEX lms_courses_product_idx (product_id)
);

CREATE TABLE IF NOT EXISTS lms_sections (
  id          VARCHAR(24) PRIMARY KEY,
  course_id   VARCHAR(24) NOT NULL,
  title       VARCHAR(160) NOT NULL,
  summary     VARCHAR(300) NOT NULL DEFAULT '',
  sort_order  INT NOT NULL DEFAULT 0,
  created_at  VARCHAR(32) NOT NULL,
  updated_at  VARCHAR(32) NOT NULL,
  INDEX lms_sections_course_idx (course_id, sort_order)
);

CREATE TABLE IF NOT EXISTS lms_lessons (
  id               VARCHAR(24) PRIMARY KEY,
  course_id        VARCHAR(24) NOT NULL,
  section_id       VARCHAR(24) NULL,
  slug             VARCHAR(100) NOT NULL,
  title            VARCHAR(160) NOT NULL,
  kind             VARCHAR(20) NOT NULL DEFAULT 'lesson',
  status           VARCHAR(20) NOT NULL DEFAULT 'published',
  summary          VARCHAR(300) NOT NULL DEFAULT '',
  content_lexical  MEDIUMTEXT NULL,
  content_html     MEDIUMTEXT NULL,
  video_url        VARCHAR(500) NOT NULL DEFAULT '',
  duration_minutes INT NOT NULL DEFAULT 0,
  preview          SMALLINT NOT NULL DEFAULT 0,
  drip_days        INT NOT NULL DEFAULT 0,
  attachments      MEDIUMTEXT NULL,
  pass_mark        INT NOT NULL DEFAULT 70,
  attempts         INT NOT NULL DEFAULT 0,
  time_limit       INT NOT NULL DEFAULT 0,
  shuffle          SMALLINT NOT NULL DEFAULT 0,
  reveal           SMALLINT NOT NULL DEFAULT 1,
  points           INT NOT NULL DEFAULT 100,
  sort_order       INT NOT NULL DEFAULT 0,
  created_at       VARCHAR(32) NOT NULL,
  updated_at       VARCHAR(32) NOT NULL,
  UNIQUE KEY lms_lessons_slug_uq (course_id, slug),
  INDEX lms_lessons_section_idx (section_id, sort_order)
);

CREATE TABLE IF NOT EXISTS lms_questions (
  id          VARCHAR(24) PRIMARY KEY,
  lesson_id   VARCHAR(24) NOT NULL,
  kind        VARCHAR(20) NOT NULL DEFAULT 'single',
  prompt      VARCHAR(1000) NOT NULL,
  options     MEDIUMTEXT NULL,
  answer      VARCHAR(300) NOT NULL DEFAULT '',
  explanation VARCHAR(1000) NOT NULL DEFAULT '',
  points      INT NOT NULL DEFAULT 1,
  sort_order  INT NOT NULL DEFAULT 0,
  created_at  VARCHAR(32) NOT NULL,
  updated_at  VARCHAR(32) NOT NULL,
  INDEX lms_questions_lesson_idx (lesson_id, sort_order)
);

CREATE TABLE IF NOT EXISTS lms_enrollments (
  id               VARCHAR(24) PRIMARY KEY,
  course_id        VARCHAR(24) NOT NULL,
  member_id        VARCHAR(24) NOT NULL,
  status           VARCHAR(20) NOT NULL DEFAULT 'active',
  source           VARCHAR(20) NOT NULL DEFAULT 'free',
  order_id         VARCHAR(24) NULL,
  progress         INT NOT NULL DEFAULT 0,
  last_lesson_id   VARCHAR(24) NULL,
  certificate_code VARCHAR(40) NULL,
  enrolled_at      VARCHAR(32) NOT NULL,
  completed_at     VARCHAR(32) NULL,
  expires_at       VARCHAR(32) NULL,
  last_active_at   VARCHAR(32) NULL,
  created_at       VARCHAR(32) NOT NULL,
  updated_at       VARCHAR(32) NOT NULL,
  UNIQUE KEY lms_enrollments_uq (course_id, member_id),
  INDEX lms_enrollments_member_idx (member_id),
  INDEX lms_enrollments_cert_idx (certificate_code)
);

CREATE TABLE IF NOT EXISTS lms_progress (
  id            VARCHAR(24) PRIMARY KEY,
  enrollment_id VARCHAR(24) NOT NULL,
  lesson_id     VARCHAR(24) NOT NULL,
  completed_at  VARCHAR(32) NOT NULL,
  UNIQUE KEY lms_progress_uq (enrollment_id, lesson_id)
);

CREATE TABLE IF NOT EXISTS lms_attempts (
  id            VARCHAR(24) PRIMARY KEY,
  enrollment_id VARCHAR(24) NOT NULL,
  lesson_id     VARCHAR(24) NOT NULL,
  score         INT NOT NULL DEFAULT 0,
  points        INT NOT NULL DEFAULT 0,
  max_points    INT NOT NULL DEFAULT 0,
  passed        SMALLINT NOT NULL DEFAULT 0,
  answers       MEDIUMTEXT NULL,
  started_at    VARCHAR(32) NULL,
  submitted_at  VARCHAR(32) NOT NULL,
  INDEX lms_attempts_idx (enrollment_id, lesson_id)
);

CREATE TABLE IF NOT EXISTS lms_submissions (
  id            VARCHAR(24) PRIMARY KEY,
  enrollment_id VARCHAR(24) NOT NULL,
  lesson_id     VARCHAR(24) NOT NULL,
  body          MEDIUMTEXT NULL,
  link          VARCHAR(500) NOT NULL DEFAULT '',
  status        VARCHAR(20) NOT NULL DEFAULT 'submitted',
  grade         INT NOT NULL DEFAULT 0,
  feedback      VARCHAR(2000) NOT NULL DEFAULT '',
  submitted_at  VARCHAR(32) NOT NULL,
  reviewed_at   VARCHAR(32) NULL,
  reviewed_by   VARCHAR(24) NULL,
  UNIQUE KEY lms_submissions_uq (enrollment_id, lesson_id),
  INDEX lms_submissions_status_idx (status, submitted_at)
);

CREATE TABLE IF NOT EXISTS lms_reviews (
  id          VARCHAR(24) PRIMARY KEY,
  course_id   VARCHAR(24) NOT NULL,
  member_id   VARCHAR(24) NOT NULL,
  rating      INT NOT NULL DEFAULT 5,
  title       VARCHAR(120) NOT NULL DEFAULT '',
  body        VARCHAR(2000) NOT NULL DEFAULT '',
  status      VARCHAR(20) NOT NULL DEFAULT 'published',
  created_at  VARCHAR(32) NOT NULL,
  updated_at  VARCHAR(32) NOT NULL,
  UNIQUE KEY lms_reviews_uq (course_id, member_id),
  INDEX lms_reviews_course_idx (course_id, status)
);

INSERT IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(RANDOM_BYTES(12))),'lms','lms_enabled','false','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'lms','lms_title','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'lms','lms_intro','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'lms','lms_base_path','courses','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'lms','lms_per_page','12','number',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'lms','lms_currency','USD','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'lms','lms_reviews','true','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'lms','lms_moderate_reviews','false','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'lms','lms_certificates','true','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'lms','lms_certificate_signer','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'lms','lms_show_students','true','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP());
