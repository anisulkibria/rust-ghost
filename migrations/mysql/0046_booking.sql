-- ============================================================================
-- 0046_booking.sql — Tools → Booking & Appointment Scheduling
-- ============================================================================
--
-- MySQL twin of migrations/sqlite/0046_booking.sql (see there for every
-- column). Long text — the editor's Lexical and HTML, plain descriptions,
-- questions, answers, notes, a staff member's hours and bio — is MEDIUMTEXT,
-- nullable because MEDIUMTEXT cannot take a DEFAULT; the repo reads those
-- columns through COALESCE.
CREATE TABLE IF NOT EXISTS booking_categories (
  id          VARCHAR(24) PRIMARY KEY,
  name        VARCHAR(80) NOT NULL,
  slug        VARCHAR(80) NOT NULL UNIQUE,
  description VARCHAR(300) NOT NULL DEFAULT '',
  tint        VARCHAR(20) NOT NULL DEFAULT 'blue',
  sort_order  INT NOT NULL DEFAULT 0,
  created_at  VARCHAR(32) NOT NULL,
  updated_at  VARCHAR(32) NOT NULL
);

CREATE TABLE IF NOT EXISTS booking_services (
  id                  VARCHAR(24) PRIMARY KEY,
  slug                VARCHAR(100) NOT NULL UNIQUE,
  name                VARCHAR(160) NOT NULL,
  category_id         VARCHAR(24) NULL,
  summary             VARCHAR(240) NOT NULL DEFAULT '',
  description         MEDIUMTEXT NULL,
  description_lexical MEDIUMTEXT NULL,
  description_html    MEDIUMTEXT NULL,
  image               VARCHAR(500) NOT NULL DEFAULT '',
  tint                VARCHAR(20) NOT NULL DEFAULT 'blue',
  duration_minutes    INT NOT NULL DEFAULT 60,
  buffer_before       INT NOT NULL DEFAULT 0,
  buffer_after        INT NOT NULL DEFAULT 0,
  slot_interval       INT NOT NULL DEFAULT 0,
  capacity            INT NOT NULL DEFAULT 1,
  price_cents         BIGINT NOT NULL DEFAULT 0,
  payment             VARCHAR(20) NOT NULL DEFAULT 'none',
  location_type       VARCHAR(20) NOT NULL DEFAULT 'in_person',
  location            VARCHAR(300) NOT NULL DEFAULT '',
  staff_mode          VARCHAR(20) NOT NULL DEFAULT 'any',
  approval            SMALLINT NOT NULL DEFAULT 0,
  min_notice          INT NOT NULL DEFAULT 120,
  max_advance         INT NOT NULL DEFAULT 60,
  external_url        VARCHAR(500) NOT NULL DEFAULT '',
  questions           MEDIUMTEXT NULL,
  status              VARCHAR(20) NOT NULL DEFAULT 'active',
  featured            SMALLINT NOT NULL DEFAULT 0,
  seo_title           VARCHAR(70) NOT NULL DEFAULT '',
  seo_description     VARCHAR(320) NOT NULL DEFAULT '',
  sort_order          INT NOT NULL DEFAULT 0,
  created_at          VARCHAR(32) NOT NULL,
  updated_at          VARCHAR(32) NOT NULL,
  INDEX booking_services_status_idx (status, sort_order)
);

CREATE TABLE IF NOT EXISTS booking_staff (
  id          VARCHAR(24) PRIMARY KEY,
  slug        VARCHAR(100) NOT NULL UNIQUE,
  name        VARCHAR(120) NOT NULL,
  title       VARCHAR(120) NOT NULL DEFAULT '',
  email       VARCHAR(191) NOT NULL DEFAULT '',
  phone       VARCHAR(40) NOT NULL DEFAULT '',
  bio         MEDIUMTEXT NULL,
  photo       VARCHAR(500) NOT NULL DEFAULT '',
  tint        VARCHAR(20) NOT NULL DEFAULT 'blue',
  hours       MEDIUMTEXT NULL,
  status      VARCHAR(20) NOT NULL DEFAULT 'active',
  sort_order  INT NOT NULL DEFAULT 0,
  created_at  VARCHAR(32) NOT NULL,
  updated_at  VARCHAR(32) NOT NULL
);

CREATE TABLE IF NOT EXISTS booking_service_staff (
  service_id  VARCHAR(24) NOT NULL,
  staff_id    VARCHAR(24) NOT NULL,
  PRIMARY KEY (service_id, staff_id)
);

CREATE TABLE IF NOT EXISTS booking_time_off (
  id          VARCHAR(24) PRIMARY KEY,
  staff_id    VARCHAR(24) NULL,
  starts_at   VARCHAR(32) NOT NULL,
  ends_at     VARCHAR(32) NOT NULL,
  reason      VARCHAR(200) NOT NULL DEFAULT '',
  created_at  VARCHAR(32) NOT NULL,
  INDEX booking_time_off_idx (starts_at, ends_at)
);

CREATE TABLE IF NOT EXISTS booking_bookings (
  id             VARCHAR(24) PRIMARY KEY,
  ref            VARCHAR(20) NOT NULL UNIQUE,
  token          VARCHAR(64) NOT NULL UNIQUE,
  service_id     VARCHAR(24) NOT NULL,
  staff_id       VARCHAR(24) NULL,
  starts_at      VARCHAR(32) NOT NULL,
  ends_at        VARCHAR(32) NOT NULL,
  seats          INT NOT NULL DEFAULT 1,
  status         VARCHAR(20) NOT NULL DEFAULT 'confirmed',
  name           VARCHAR(120) NOT NULL,
  email          VARCHAR(191) NOT NULL,
  phone          VARCHAR(40) NOT NULL DEFAULT '',
  notes          MEDIUMTEXT NULL,
  answers        MEDIUMTEXT NULL,
  timezone       VARCHAR(64) NOT NULL DEFAULT '',
  price_cents    BIGINT NOT NULL DEFAULT 0,
  payment_status VARCHAR(20) NOT NULL DEFAULT 'none',
  payment_ref    VARCHAR(120) NOT NULL DEFAULT '',
  member_id      VARCHAR(24) NULL,
  source         VARCHAR(20) NOT NULL DEFAULT 'site',
  cancel_reason  VARCHAR(300) NOT NULL DEFAULT '',
  internal_note  MEDIUMTEXT NULL,
  reminded_at    VARCHAR(32) NULL,
  created_at     VARCHAR(32) NOT NULL,
  updated_at     VARCHAR(32) NOT NULL,
  INDEX booking_bookings_time_idx (starts_at, status),
  INDEX booking_bookings_staff_idx (staff_id, starts_at),
  INDEX booking_bookings_email_idx (email)
);

INSERT IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(RANDOM_BYTES(12))),'booking','booking_enabled','false','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'booking','booking_title','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'booking','booking_intro','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'booking','booking_base_path','book','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'booking','booking_timezone','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'booking','booking_hours','{"mon":"09:00-17:00","tue":"09:00-17:00","wed":"09:00-17:00","thu":"09:00-17:00","fri":"09:00-17:00","sat":"closed","sun":"closed"}','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'booking','booking_cancel_hours','24','number',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'booking','booking_reminder_hours','24','number',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'booking','booking_notify_email','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'booking','booking_currency','USD','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'booking','booking_require_phone','false','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'booking','booking_terms','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'booking','booking_address','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP());
