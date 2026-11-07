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
