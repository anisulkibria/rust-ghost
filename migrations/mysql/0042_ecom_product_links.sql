-- ============================================================================
-- 0042_ecom_product_links.sql — Tools → E-commerce: a product's outside links
-- ============================================================================
--
-- MySQL twin of migrations/sqlite/0042_ecom_product_links.sql (see there).
-- VARCHAR, not TEXT: the Any driver cannot read TEXT back on MySQL.
ALTER TABLE ecom_products ADD COLUMN demo_url VARCHAR(500) NOT NULL DEFAULT '';
ALTER TABLE ecom_products ADD COLUMN purchase_url VARCHAR(500) NOT NULL DEFAULT '';
