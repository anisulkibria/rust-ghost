-- Least privilege for the two internal integrations.
--
-- Both were seeded with the broad "Admin Integration" role, which grants
-- almost everything: posts, members, settings, themes, users, webhooks. The
-- scheduler needs to publish a post. The backup needs to read and export.
-- Neither needs the rest, and both keys are readable from the database and
-- visible on the integrations screen — so the difference is the blast radius
-- if one gets out.
--
-- These are Ghost's own roles for these integrations
-- (`services/permissions/permissions-data.js`). Matched here so a key that
-- leaks can do only what its integration was ever for.
--
-- Scoped by integration slug, so a custom integration an operator
-- deliberately gave the admin role keeps it.
UPDATE api_keys
SET role_id = (SELECT id FROM roles WHERE name = 'Scheduler Integration'),
    updated_at = UTC_TIMESTAMP()
WHERE type = 'admin'
  AND integration_id IN (SELECT id FROM integrations WHERE slug = 'ghost-scheduler')
  AND EXISTS (SELECT 1 FROM roles WHERE name = 'Scheduler Integration');

UPDATE api_keys
SET role_id = (SELECT id FROM roles WHERE name = 'DB Backup Integration'),
    updated_at = UTC_TIMESTAMP()
WHERE type = 'admin'
  AND integration_id IN (SELECT id FROM integrations WHERE slug = 'ghost-backup')
  AND EXISTS (SELECT 1 FROM roles WHERE name = 'DB Backup Integration');
