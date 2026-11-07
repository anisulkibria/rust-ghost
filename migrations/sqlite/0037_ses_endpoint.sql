-- Mail Manager SMTP endpoint; blank keeps regional SES SMTP behavior.
INSERT INTO settings (id, `group`, key, value, type, created_at, updated_at)
SELECT lower(hex(randomblob(12))), 'email', 'ses_smtp_endpoint', '', 'string', datetime('now'), datetime('now')
WHERE NOT EXISTS (SELECT 1 FROM settings WHERE key = 'ses_smtp_endpoint');
