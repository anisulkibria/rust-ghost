-- ============================================================================
-- 0044_directory_rich_listings.sql — Tools → Directory: real listings
-- ============================================================================
--
-- What the popular directories (maps, city guides, review sites) ask a
-- business for, added to `directory_listings`:
--
--   description_lexical / description_html
--               the description written in the post editor (Koenig). The
--               old plain `description` stays, kept as the plain-text copy
--               search and cards read; a public submission still fills only
--               the plain text.
--   logo        an image address, like `image`.
--   gallery     more images, one address per line. `image` stays the main one.
--   region, postal_code, country
--               the rest of the postal address (`location` is the town).
--   latitude, longitude
--               decimal degrees as text, or empty.
--   booking_url, video_url
--               http(s) or empty.
--   hours       JSON object, day key (mon … sun) → `closed`, `24h`, or
--               `HH:MM-HH:MM` ranges separated by commas.
--   hours_note  one line ("Kitchen closes an hour earlier").
--   price_level 0 (not set) … 4 ($ … $$$$).
--   amenities, payments
--               comma-separated, from fixed lists in vuuut_core::directory.
--   socials     JSON object, network → profile address.
--   established a year, or empty.
--   seo_title, seo_description
--               the search-engine listing; empty falls back to the title and
--               the summary.
--
-- `directory_categories.image` — a picture for the category tile themes show.
ALTER TABLE directory_listings ADD COLUMN description_lexical TEXT NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN description_html TEXT NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN logo TEXT NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN gallery TEXT NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN region TEXT NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN postal_code TEXT NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN country TEXT NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN latitude TEXT NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN longitude TEXT NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN booking_url TEXT NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN video_url TEXT NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN hours TEXT NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN hours_note TEXT NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN price_level INTEGER NOT NULL DEFAULT 0;
ALTER TABLE directory_listings ADD COLUMN amenities TEXT NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN payments TEXT NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN socials TEXT NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN established TEXT NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN seo_title TEXT NOT NULL DEFAULT '';
ALTER TABLE directory_listings ADD COLUMN seo_description TEXT NOT NULL DEFAULT '';
ALTER TABLE directory_categories ADD COLUMN image TEXT NOT NULL DEFAULT '';
