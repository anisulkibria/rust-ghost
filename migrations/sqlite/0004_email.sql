-- Email: transport settings, newsletter design, open tracking, batching, SES and
-- the send-volume ledger.
--
-- Consolidated from earlier migrations. Each section below is one of them (or the
-- named part of one), unchanged and in the order it was first applied.

-- ============================================================================
-- mail_settings  (was 0004_mail_settings.sql)
-- ============================================================================

-- Mail configuration, held in settings rather than a config file.
--
-- Ghost keeps the transactional transport in `config.<env>.json` under `mail`
-- and the bulk provider under `bulkEmail`, so changing either means editing a
-- file and restarting. This port edits both from Settings → Email and
-- Settings → Bulk email, which means they have to be rows.
--
-- `SettingsCache::set` is an UPDATE, not an upsert — an unknown key is a
-- hard error — so every key the UI writes must exist up front, including the
-- ones whose default is empty.
--
-- `mailgun_*` already exist from the initial schema; only the new keys are
-- inserted here. Credential rows are plain strings and are masked by the
-- Admin API, absent from the theme/Content API allowlist, and never rendered
-- back into the settings form.
INSERT INTO settings (id, `group`, key, value, type, created_at, updated_at)
SELECT lower(hex(randomblob(12))), g, k, v, 'string', datetime('now'), datetime('now')
FROM (
    SELECT 'email' AS g, 'mail_transport'          AS k, ''    AS v UNION ALL
    SELECT 'email',       'mail_smtp_host',               ''         UNION ALL
    SELECT 'email',       'mail_smtp_port',               '587'      UNION ALL
    SELECT 'email',       'mail_smtp_security',           'starttls' UNION ALL
    SELECT 'email',       'mail_smtp_user',               ''         UNION ALL
    SELECT 'email',       'mail_smtp_pass',               ''         UNION ALL
    SELECT 'email',       'mail_from_address',            ''         UNION ALL
    SELECT 'email',       'mail_from_name',               ''         UNION ALL
    SELECT 'email',       'mail_reply_to',                ''         UNION ALL
    SELECT 'bulk_email',  'bulk_email_from_address',      ''
) AS defaults
WHERE NOT EXISTS (SELECT 1 FROM settings s WHERE s.key = defaults.k);

-- ============================================================================
-- mail_transport_direct  (was 0005_mail_transport_direct.sql)
-- ============================================================================

-- Default the transactional transport to Direct, matching Ghost.
--
-- Ghost's mailer reads `config.get('mail').transport || 'direct'`, so a site
-- with no mail block sends via nodemailer's direct transport: an MX lookup
-- on the recipient's domain, then SMTP straight to their mail server with no
-- relay and no credentials. This port now does the same, so the stored
-- setting should say so rather than being blank.
--
-- Only blank rows are touched — a site that has already chosen SMTP keeps it.
UPDATE settings
SET value = 'Direct', updated_at = datetime('now')
WHERE key = 'mail_transport' AND (value IS NULL OR trim(value) = '');

-- ============================================================================
-- newsletter_design  (was 0007_newsletter_design.sql)
-- ============================================================================

-- Newsletter design settings.
--
-- 0001 created `newsletters` with the columns Ghost had at the time. Ghost 6
-- added typography and shape settings on top, and the email template reads
-- all of them, so the table needs the rest of the set. Names, types and
-- defaults are copied from `core/server/data/schema/schema.js` so a real
-- Ghost database still imports cleanly.
--
-- SQLite's ALTER TABLE ADD COLUMN is cheap and does not rewrite the table.
-- There is no IF NOT EXISTS for it, which is fine: sqlx records the migration
-- and never reruns it.

-- Typography. `title_*` covers the post title and headings; `body_*` the
-- post body.
ALTER TABLE newsletters ADD COLUMN title_font_category TEXT(191) NOT NULL DEFAULT 'sans_serif';
ALTER TABLE newsletters ADD COLUMN title_alignment TEXT(191) NOT NULL DEFAULT 'center';
ALTER TABLE newsletters ADD COLUMN title_font_weight TEXT(50) NOT NULL DEFAULT 'bold';
ALTER TABLE newsletters ADD COLUMN body_font_category TEXT(191) NOT NULL DEFAULT 'sans_serif';

-- Links and buttons. `link_color`/`button_color` already exist and hold
-- either the sentinel 'accent' or a '#rrggbb' value; these add the styling
-- that goes with them.
ALTER TABLE newsletters ADD COLUMN link_style TEXT(50) NOT NULL DEFAULT 'underline';
ALTER TABLE newsletters ADD COLUMN button_style TEXT(50) NOT NULL DEFAULT 'fill';
ALTER TABLE newsletters ADD COLUMN button_corners TEXT(50) NOT NULL DEFAULT 'rounded';

-- Image corners: 'square' or 'rounded'.
ALTER TABLE newsletters ADD COLUMN image_corners TEXT(50) NOT NULL DEFAULT 'square';

-- ============================================================================
-- newsletter_named_after_site  (was 0008_newsletter_named_after_site.sql)
-- ============================================================================

-- Name the default newsletter after the site.
--
-- Ghost's fixture calls it "Default Newsletter" and ships with
-- `show_header_name` off, so upstream nobody ever reads that name. This port
-- shows it in the email header, where "Default Newsletter" under the site
-- title is plainly a leftover from a fixture file — a subscriber should see
-- the publication they subscribed to.
--
-- From here on the name is derived from the site title (see
-- `repos::newsletters::default_name`, and `SettingsCache::set`, which keeps
-- the two in step when the title changes). This brings existing installs in
-- line; a newsletter anyone has already named is matched by neither
-- condition and keeps its name.

UPDATE newsletters
SET name = TRIM(COALESCE((SELECT value FROM settings WHERE key = 'title'), '')) || ' Newsletter',
    updated_at = STRFTIME('%Y-%m-%d %H:%M:%S', 'now')
WHERE name = 'Default Newsletter'
  -- An empty or missing title would leave the name as " Newsletter", which
  -- is worse than the fixture name it replaces.
  AND TRIM(COALESCE((SELECT value FROM settings WHERE key = 'title'), '')) <> '';

-- ============================================================================
-- email_open_tracking  (was 0024_email_open_tracking.sql)
-- ============================================================================

-- Per-recipient newsletter delivery records.
--
-- `email_recipients` has been in the schema since the first migration but
-- nothing ever wrote to it, so the open rate and delivered counts the
-- Analytics → Newsletters tab reads out of it were always zero. Sending now
-- writes a row per recipient, and these columns are what the open pixel and
-- the Mailgun webhook match that row on.
--
-- `member_uuid` is stored rather than joined for, as Ghost stores it: the
-- pixel is identified by uuid, and the record should survive the member being
-- deleted. `mailgun_message_id` is Ghost's name for the provider's id for the
-- message; Ghost keeps it on `email_batches` because it sends in batches of a
-- thousand, while this port sends one request per recipient, so the id
-- belongs to the recipient row here.
ALTER TABLE email_recipients ADD COLUMN member_uuid TEXT(36);
ALTER TABLE email_recipients ADD COLUMN mailgun_message_id TEXT(191);

CREATE INDEX IF NOT EXISTS email_recipients_uuid_idx ON email_recipients (email_id, member_uuid);
CREATE INDEX IF NOT EXISTS email_recipients_provider_idx ON email_recipients (mailgun_message_id);

-- ============================================================================
-- email_batching  (was 0025_email_batching.sql)
-- ============================================================================

-- Batched newsletter sending, and the records that make a send inspectable.
--
-- `email_batches` and `email_recipient_failures` have been in the schema
-- since the first migration with nothing reading or writing them, because
-- sending was one synchronous Mailgun request per recipient. These columns
-- are the ones Ghost's own batch sending keeps, so a send can be reported on
-- and the recipients it missed can be retried.
--
-- Not added: `emails.csd_email_count`. It counts messages sent through a
-- custom sending domain while that domain is being warmed up, which belongs
-- to a domain-warming service this port does not have.

-- A send progresses pending → submitting → submitted/failed, and the counts
-- say how many recipients were considered at each step: `candidate_count`
-- before access and suppression rules, `preflight_email_count` after them,
-- `preparation_excluded_count` the difference.
ALTER TABLE emails ADD COLUMN uuid TEXT(36);
ALTER TABLE emails ADD COLUMN source_type TEXT(50);
ALTER TABLE emails ADD COLUMN candidate_count INTEGER UNSIGNED;
ALTER TABLE emails ADD COLUMN preflight_email_count INTEGER UNSIGNED;
ALTER TABLE emails ADD COLUMN preparation_excluded_count INTEGER UNSIGNED;
ALTER TABLE emails ADD COLUMN prepared_at TEXT;

-- One row per provider request. `member_segment` records which body variant
-- the batch carried, since a paywalled post goes out as more than one.
ALTER TABLE email_batches ADD COLUMN member_segment TEXT(2000);
ALTER TABLE email_batches ADD COLUMN recipient_count INTEGER UNSIGNED NOT NULL DEFAULT 0;
ALTER TABLE email_batches ADD COLUMN submitted_count INTEGER UNSIGNED NOT NULL DEFAULT 0;
ALTER TABLE email_batches ADD COLUMN error_status_code INTEGER;
ALTER TABLE email_batches ADD COLUMN error_message TEXT(2000);

-- Why a particular recipient did not get the email. `severity` is Mailgun's
-- `permanent`/`temporary`: a temporary failure is worth retrying, a
-- permanent one is not.
ALTER TABLE email_recipient_failures ADD COLUMN batch_id TEXT(24);
ALTER TABLE email_recipient_failures ADD COLUMN code INTEGER;
ALTER TABLE email_recipient_failures ADD COLUMN enhanced_code TEXT(50);
ALTER TABLE email_recipient_failures ADD COLUMN severity TEXT(50);
ALTER TABLE email_recipient_failures ADD COLUMN event_id TEXT(191);

CREATE INDEX IF NOT EXISTS email_batches_email_idx ON email_batches (email_id);
CREATE INDEX IF NOT EXISTS email_recipient_failures_email_idx ON email_recipient_failures (email_id);
CREATE UNIQUE INDEX IF NOT EXISTS email_recipient_failures_event_idx ON email_recipient_failures (event_id);
CREATE INDEX IF NOT EXISTS email_recipients_batch_idx ON email_recipients (batch_id);

-- ============================================================================
-- ses_mail  (was 0035_ses_mail.sql)
-- ============================================================================

-- SES SMTP configuration and explicit newsletter provider selection.
INSERT INTO settings (id, `group`, key, value, type, created_at, updated_at)
SELECT lower(hex(randomblob(12))), g, k, v, 'string', datetime('now'), datetime('now')
FROM (
    SELECT 'email' AS g, 'ses_region' AS k, '' AS v UNION ALL
    SELECT 'email', 'ses_smtp_user', '' UNION ALL
    SELECT 'email', 'ses_smtp_pass', '' UNION ALL
    SELECT 'email', 'ses_max_send_rate', '1' UNION ALL
    SELECT 'bulk_email', 'newsletter_transport', 'auto'
) AS defaults
WHERE NOT EXISTS (SELECT 1 FROM settings s WHERE s.key = defaults.k);

-- ============================================================================
-- email_volume  (was 0036_email_volume.sql)
-- ============================================================================

-- Actual provider submissions, shared by transactional and newsletter mail.
CREATE TABLE mail_send_events (
    id TEXT PRIMARY KEY,
    period_start TEXT NOT NULL,
    kind TEXT NOT NULL,
    provider TEXT NOT NULL,
    recipient_count BIGINT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX mail_send_events_created ON mail_send_events(created_at);
CREATE INDEX mail_send_events_period ON mail_send_events(period_start, status);
CREATE TABLE mail_send_periods (
    period_start TEXT PRIMARY KEY,
    sent BIGINT NOT NULL DEFAULT 0,
    reserved BIGINT NOT NULL DEFAULT 0
);
-- Preserve known newsletter submissions; historical transactional mail was not logged.
INSERT INTO mail_send_events (id,period_start,kind,provider,recipient_count,status,created_at)
SELECT 'legacy-' || id, '', 'newsletter', 'legacy', delivered_count, 'sent', strftime('%Y-%m-%dT%H:%M:%SZ', created_at)
FROM emails WHERE delivered_count > 0 AND created_at IS NOT NULL;

-- ============================================================================
-- ses_endpoint  (was 0037_ses_endpoint.sql)
-- ============================================================================

-- Mail Manager SMTP endpoint; blank keeps regional SES SMTP behavior.
INSERT INTO settings (id, `group`, key, value, type, created_at, updated_at)
SELECT lower(hex(randomblob(12))), 'email', 'ses_smtp_endpoint', '', 'string', datetime('now'), datetime('now')
WHERE NOT EXISTS (SELECT 1 FROM settings WHERE key = 'ses_smtp_endpoint');
