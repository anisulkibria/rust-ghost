-- ============================================================================
-- 0023_cookies.sql — Cookie consent & privacy compliance
-- ============================================================================
--
-- Two tables and thirteen settings rows.
--
-- `cookie_declarations` is the site's list of the cookies it sets, beyond the
-- ones the CMS sets itself (those are `ghost_core::cookies::builtin_cookies`,
-- not rows, because they are what the code does rather than what somebody
-- typed). Visitors read it in the banner's preferences panel and wherever an
-- author places `<div data-cookie-declaration></div>`.
--
--   category    necessary | preferences | analytics | marketing
--   tracker_key the `ghost_core::cookies::TRACKERS` entry it was added from,
--               or '' when typed by hand. Lets the scan tell which trackers
--               are already declared.
--
-- `cookie_consents` is the proof-of-consent log — GDPR Art. 7(1) asks the site
-- to be able to show that somebody agreed. It is deliberately thin: no IP
-- address, no user agent, no member id. A row is a random id the visitor's
-- own browser generated (and can show them), what they chose, under which
-- revision of the banner, and when. That is enough to answer "did this
-- person agree?" and not enough to identify anybody.
--
--   action      accept_all | reject_all | custom | gpc | acknowledge
--   categories  comma list in fixed order, always starting `necessary`
--   revision    the `cookies_revision` the decision was made under
--   gpc         1 when the request carried Sec-GPC: 1
--
-- Rows older than `cookies_log_days` are deleted by the module itself.
CREATE TABLE IF NOT EXISTS cookie_declarations (
  id          TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  provider    TEXT NOT NULL DEFAULT '',
  category    TEXT NOT NULL DEFAULT 'necessary',
  duration    TEXT NOT NULL DEFAULT '',
  purpose     TEXT NOT NULL DEFAULT '',
  tracker_key TEXT NOT NULL DEFAULT '',
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS cookie_consents (
  id         TEXT PRIMARY KEY,
  consent_id TEXT NOT NULL,
  action     TEXT NOT NULL,
  categories TEXT NOT NULL,
  revision   INTEGER NOT NULL DEFAULT 1,
  gpc        INTEGER NOT NULL DEFAULT 0,
  page       TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL
);

-- The figures read a window by date; the log's search reads by consent id;
-- the retention sweep deletes by date.
CREATE INDEX IF NOT EXISTS cookie_consents_created_idx ON cookie_consents (created_at);
CREATE INDEX IF NOT EXISTS cookie_consents_consent_idx ON cookie_consents (consent_id);

-- Settings → Cookies. The banner ships OFF: a consent banner is a promise the
-- site owner has to choose to make, so installing this changes nothing a
-- visitor sees. Every other default is the stricter reading of the law —
-- ask first, honour GPC, keep a record — so switching it on is enough.
INSERT OR IGNORE INTO settings (id,`group`,key,value,type,created_at,updated_at) VALUES
(lower(hex(randomblob(12))),'cookies','cookies_enabled','false','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'cookies','cookies_mode','opt_in','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'cookies','cookies_layout','box','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'cookies','cookies_title','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'cookies','cookies_message','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'cookies','cookies_policy_url','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'cookies','cookies_revision','1','number',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'cookies','cookies_expiry_months','12','number',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'cookies','cookies_honour_gpc','true','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'cookies','cookies_block_embeds','false','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'cookies','cookies_reopen','true','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'cookies','cookies_log','true','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'cookies','cookies_log_days','365','number',datetime('now'),datetime('now'));
