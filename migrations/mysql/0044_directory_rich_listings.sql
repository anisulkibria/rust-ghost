-- ============================================================================
-- 0044_directory_rich_listings.sql — Tools → Directory: real listings
-- ============================================================================
--
-- MySQL twin of migrations/sqlite/0044_directory_rich_listings.sql (see there).
-- The editor's Lexical and HTML, the gallery, hours and socials are
-- MEDIUMTEXT like ecom_products' description (they do not count toward the
-- row size); everything else is VARCHAR. MEDIUMTEXT cannot take a DEFAULT,
-- so those columns are nullable and the repo reads them through COALESCE.
ALTER TABLE directory_listings ADD COLUMN description_lexical MEDIUMTEXT NULL;
ALTER TABLE directory_listings ADD COLUMN description_html MEDIUMTEXT NULL;
ALTER TABLE directory_listings ADD COLUMN logo VARCHAR(500) NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN gallery MEDIUMTEXT NULL;
ALTER TABLE directory_listings ADD COLUMN region VARCHAR(80) NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN postal_code VARCHAR(20) NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN country VARCHAR(80) NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN latitude VARCHAR(24) NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN longitude VARCHAR(24) NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN booking_url VARCHAR(500) NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN video_url VARCHAR(500) NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN hours MEDIUMTEXT NULL;
ALTER TABLE directory_listings ADD COLUMN hours_note VARCHAR(160) NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN price_level SMALLINT NOT NULL DEFAULT 0;
ALTER TABLE directory_listings ADD COLUMN amenities VARCHAR(1000) NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN payments VARCHAR(300) NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN socials MEDIUMTEXT NULL;
ALTER TABLE directory_listings ADD COLUMN established VARCHAR(10) NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN seo_title VARCHAR(70) NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN seo_description VARCHAR(320) NOT NULL DEFAULT '';
ALTER TABLE directory_categories ADD COLUMN image VARCHAR(500) NOT NULL DEFAULT '';
