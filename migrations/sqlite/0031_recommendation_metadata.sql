-- Recommendation display metadata.
--
-- `POST /recommendations/check` already reads a candidate publication's
-- favicon, social image and excerpt out of its HTML, but there was nowhere to
-- keep them, so every saved recommendation rendered as a bare title and link.
-- Ghost stores all three on the row (`core/server/data/schema/schema.js`,
-- `recommendations`) and its theme helper and Portal list both read them.
ALTER TABLE recommendations ADD COLUMN excerpt TEXT(2000);
ALTER TABLE recommendations ADD COLUMN featured_image TEXT(2000);
ALTER TABLE recommendations ADD COLUMN favicon TEXT(2000);
