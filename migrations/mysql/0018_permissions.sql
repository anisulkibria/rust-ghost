-- ============================================================================
-- 0018_permissions.sql — what each staff role is allowed to reach
-- ============================================================================
--
-- The MySQL twin of migrations/sqlite/0018_permissions.sql; read that one for
-- why this is a single settings row holding the whole role × resource matrix
-- as JSON, why it ships empty, and why the Owner is not in it.
INSERT IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(RANDOM_BYTES(12))),'permissions','permissions','{}','object',UTC_TIMESTAMP(),UTC_TIMESTAMP());
