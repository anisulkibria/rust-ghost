-- ============================================================================
-- 0019_popups.sql — popups, and the leads they capture
-- ============================================================================
--
-- Two tables, because they answer two different questions and are written at
-- very different rates: `popups` is edited by staff a few times a campaign,
-- and `popup_leads` gains a row every time a visitor hands over an address.
--
-- Why the counters live on `popups` rather than in an events table:
-- an impression happens on every matching page view, and a row per impression
-- would be the busiest table on the site to answer one number per popup. The
-- counters are incremented in place and are deliberately approximate — a lost
-- increment costs a decimal place on a conversion rate, and nothing else
-- reads them. A lead, by contrast, is a person who asked for something and
-- gets a row of its own.
--
-- `member_id` is nullable and is NOT a foreign key: a lead is the record of
-- somebody asking, and it has to survive the member being deleted — that is
-- the whole point of keeping it. It is filled in when the signup succeeded.
CREATE TABLE IF NOT EXISTS popups (
  id              TEXT PRIMARY KEY,
  name            TEXT NOT NULL,
  status          TEXT NOT NULL DEFAULT 'draft',   -- draft | live
  kind            TEXT NOT NULL DEFAULT 'modal',   -- modal | slide_in | bar
  heading         TEXT NOT NULL DEFAULT '',
  body            TEXT NOT NULL DEFAULT '',
  button_label    TEXT NOT NULL DEFAULT '',
  button_url      TEXT NOT NULL DEFAULT '',        -- blank when it captures an address
  success_message TEXT NOT NULL DEFAULT '',
  capture_email   INTEGER NOT NULL DEFAULT 1,
  capture_name    INTEGER NOT NULL DEFAULT 0,
  trigger_kind    TEXT NOT NULL DEFAULT 'delay',   -- delay | scroll | exit
  trigger_value   INTEGER NOT NULL DEFAULT 5,
  frequency       TEXT NOT NULL DEFAULT 'once',    -- once | days | always
  frequency_days  INTEGER NOT NULL DEFAULT 0,
  audience        TEXT NOT NULL DEFAULT 'logged_out',
  targeting       TEXT NOT NULL DEFAULT 'everywhere',
  paths           TEXT NOT NULL DEFAULT '',        -- one prefix per line
  starts_at       TEXT,
  ends_at         TEXT,
  impressions     INTEGER NOT NULL DEFAULT 0,
  submissions     INTEGER NOT NULL DEFAULT 0,
  created_at      TEXT NOT NULL,
  updated_at      TEXT NOT NULL
);

-- The public endpoint asks one question — "which popups are live?" — on
-- requests that are already hot, so that one question gets the index.
CREATE INDEX IF NOT EXISTS popups_status_idx ON popups (status);

CREATE TABLE IF NOT EXISTS popup_leads (
  id         TEXT PRIMARY KEY,
  popup_id   TEXT NOT NULL,
  email      TEXT NOT NULL,
  name       TEXT NOT NULL DEFAULT '',
  source_url TEXT NOT NULL DEFAULT '',
  member_id  TEXT,
  created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS popup_leads_popup_idx ON popup_leads (popup_id, created_at);
-- Finding whether this address already came through a given popup is what
-- stops one person being counted as ten conversions.
CREATE INDEX IF NOT EXISTS popup_leads_email_idx ON popup_leads (popup_id, email);
