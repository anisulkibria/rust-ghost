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
