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
