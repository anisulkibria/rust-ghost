-- ============================================================================
-- 0020_popup_surface.sql — which KIND of page a popup belongs on
-- ============================================================================
--
-- The MySQL twin of migrations/sqlite/0020_popup_surface.sql; read that one
-- for why this is a second migration rather than an edit to 0019, and why it
-- is ANDed with the path rules rather than replacing them.
ALTER TABLE popups ADD COLUMN surface VARCHAR(20) NOT NULL DEFAULT 'any';
