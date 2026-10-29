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
ALTER TABLE emails ADD COLUMN prepared_at DATETIME;

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
