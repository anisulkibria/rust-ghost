-- ============================================================================
-- 0017_backups.sql — when the site backs itself up
-- ============================================================================
--
-- Backups themselves are files under content/backups/ (see
-- crates/ghost-api/src/backups.rs), not rows: a backup has to survive the
-- database being replaced by an older one, which is what restoring does.
-- What lives here is only the schedule: 'off' | 'daily' | 'weekly' | 'monthly'.
INSERT IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(RANDOM_BYTES(12))),'backups','backup_schedule','off','string',UTC_TIMESTAMP(),UTC_TIMESTAMP());
