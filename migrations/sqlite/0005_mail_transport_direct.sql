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
