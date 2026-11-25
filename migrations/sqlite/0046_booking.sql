-- ============================================================================
-- 0046_booking.sql — Tools → Booking & Appointment Scheduling
-- ============================================================================
--
-- What Calendly, Acuity, Bookly, Amelia, SimplyBook and Square Appointments
-- have in common: services with a length and a price, the people who give
-- them and their weekly hours, time off, and bookings made on the site.
--
-- `booking_categories` — how services are grouped on the booking page.
--
-- `booking_services` — something that can be booked.
--   duration_minutes, buffer_before, buffer_after   in minutes
--   slot_interval   minutes between start times; 0 = the service's length
--   capacity        1 = an appointment; more = a class or group session
--   location_type   in_person | video | phone | customer (at their address)
--   payment         none (free or pay later) | onsite (pay at the visit) |
--                   online (Stripe at booking; the slot is held meanwhile)
--   staff_mode      any (first free person) | choose (the customer picks)
--   approval        bookings wait for staff to confirm
--   min_notice      minutes before a slot that it can still be booked
--   max_advance     days ahead that can be booked
--   external_url    book on another system (Calendly, Acuity, Cal.com…);
--                   the site links there instead of taking the booking
--   questions       JSON [{"label","kind","required"}] asked on the form
--   status          active | hidden (bookable by link only) | draft
--
-- `booking_staff` — a person (or a room, a court, a chair) that is booked.
--   hours           JSON week, day key → `closed` | `24h` | `HH:MM-HH:MM,…`
--                   (vuuut_core::directory::Hours); empty days use the
--                   business hours from settings
--
-- `booking_service_staff` — who gives what.
-- `booking_time_off` — a blocked span (UTC), for one person or, with no
--   staff_id, for the whole business (holidays).
--
-- `booking_bookings` — one booking.
--   ref             short public reference (BK-7H2Q-9XKM)
--   token           secret for the customer's manage page
--   starts_at, ends_at   UTC, the service time without buffers
--   status          awaiting_payment | pending | confirmed | completed |
--                   cancelled | no_show
--   payment_status  none | unpaid | paid | refunded
--   answers         JSON [{"label","value"}]
--   timezone        the customer's, for showing times back to them
--
-- Ships switched off: no booking page until somebody turns it on.
CREATE TABLE IF NOT EXISTS booking_categories (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  slug        TEXT NOT NULL UNIQUE,
  description TEXT NOT NULL DEFAULT '',
  tint        TEXT NOT NULL DEFAULT 'blue',
  sort_order  INTEGER NOT NULL DEFAULT 0,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS booking_services (
  id                  TEXT PRIMARY KEY,
  slug                TEXT NOT NULL UNIQUE,
  name                TEXT NOT NULL,
  category_id         TEXT,
  summary             TEXT NOT NULL DEFAULT '',
  description         TEXT NOT NULL DEFAULT '',
  description_lexical TEXT NOT NULL DEFAULT '',
  description_html    TEXT NOT NULL DEFAULT '',
  image               TEXT NOT NULL DEFAULT '',
  tint                TEXT NOT NULL DEFAULT 'blue',
  duration_minutes    INTEGER NOT NULL DEFAULT 60,
  buffer_before       INTEGER NOT NULL DEFAULT 0,
  buffer_after        INTEGER NOT NULL DEFAULT 0,
  slot_interval       INTEGER NOT NULL DEFAULT 0,
  capacity            INTEGER NOT NULL DEFAULT 1,
  price_cents         INTEGER NOT NULL DEFAULT 0,
  payment             TEXT NOT NULL DEFAULT 'none',
  location_type       TEXT NOT NULL DEFAULT 'in_person',
  location            TEXT NOT NULL DEFAULT '',
  staff_mode          TEXT NOT NULL DEFAULT 'any',
  approval            INTEGER NOT NULL DEFAULT 0,
  min_notice          INTEGER NOT NULL DEFAULT 120,
  max_advance         INTEGER NOT NULL DEFAULT 60,
  external_url        TEXT NOT NULL DEFAULT '',
  questions           TEXT NOT NULL DEFAULT '',
  status              TEXT NOT NULL DEFAULT 'active',
  featured            INTEGER NOT NULL DEFAULT 0,
  seo_title           TEXT NOT NULL DEFAULT '',
  seo_description     TEXT NOT NULL DEFAULT '',
  sort_order          INTEGER NOT NULL DEFAULT 0,
  created_at          TEXT NOT NULL,
  updated_at          TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS booking_services_status_idx ON booking_services (status, sort_order);

CREATE TABLE IF NOT EXISTS booking_staff (
  id          TEXT PRIMARY KEY,
  slug        TEXT NOT NULL UNIQUE,
  name        TEXT NOT NULL,
  title       TEXT NOT NULL DEFAULT '',
  email       TEXT NOT NULL DEFAULT '',
  phone       TEXT NOT NULL DEFAULT '',
  bio         TEXT NOT NULL DEFAULT '',
  photo       TEXT NOT NULL DEFAULT '',
  tint        TEXT NOT NULL DEFAULT 'blue',
  hours       TEXT NOT NULL DEFAULT '',
  status      TEXT NOT NULL DEFAULT 'active',
  sort_order  INTEGER NOT NULL DEFAULT 0,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS booking_service_staff (
  service_id  TEXT NOT NULL,
  staff_id    TEXT NOT NULL,
  PRIMARY KEY (service_id, staff_id)
);

CREATE TABLE IF NOT EXISTS booking_time_off (
  id          TEXT PRIMARY KEY,
  staff_id    TEXT,
  starts_at   TEXT NOT NULL,
  ends_at     TEXT NOT NULL,
  reason      TEXT NOT NULL DEFAULT '',
  created_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS booking_time_off_idx ON booking_time_off (starts_at, ends_at);

CREATE TABLE IF NOT EXISTS booking_bookings (
  id             TEXT PRIMARY KEY,
  ref            TEXT NOT NULL UNIQUE,
  token          TEXT NOT NULL UNIQUE,
  service_id     TEXT NOT NULL,
  staff_id       TEXT,
  starts_at      TEXT NOT NULL,
  ends_at        TEXT NOT NULL,
  seats          INTEGER NOT NULL DEFAULT 1,
  status         TEXT NOT NULL DEFAULT 'confirmed',
  name           TEXT NOT NULL,
  email          TEXT NOT NULL,
  phone          TEXT NOT NULL DEFAULT '',
  notes          TEXT NOT NULL DEFAULT '',
  answers        TEXT NOT NULL DEFAULT '',
  timezone       TEXT NOT NULL DEFAULT '',
  price_cents    INTEGER NOT NULL DEFAULT 0,
  payment_status TEXT NOT NULL DEFAULT 'none',
  payment_ref    TEXT NOT NULL DEFAULT '',
  member_id      TEXT,
  source         TEXT NOT NULL DEFAULT 'site',
  cancel_reason  TEXT NOT NULL DEFAULT '',
  internal_note  TEXT NOT NULL DEFAULT '',
  reminded_at    TEXT,
  created_at     TEXT NOT NULL,
  updated_at     TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS booking_bookings_time_idx ON booking_bookings (starts_at, status);
CREATE INDEX IF NOT EXISTS booking_bookings_staff_idx ON booking_bookings (staff_id, starts_at);
CREATE INDEX IF NOT EXISTS booking_bookings_email_idx ON booking_bookings (email);

INSERT OR IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(randomblob(12))),'booking','booking_enabled','false','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'booking','booking_title','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'booking','booking_intro','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'booking','booking_base_path','book','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'booking','booking_timezone','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'booking','booking_hours','{"mon":"09:00-17:00","tue":"09:00-17:00","wed":"09:00-17:00","thu":"09:00-17:00","fri":"09:00-17:00","sat":"closed","sun":"closed"}','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'booking','booking_cancel_hours','24','number',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'booking','booking_reminder_hours','24','number',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'booking','booking_notify_email','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'booking','booking_currency','USD','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'booking','booking_require_phone','false','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'booking','booking_terms','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'booking','booking_address','','string',datetime('now'),datetime('now'));
