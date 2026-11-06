-- Newsletter design settings.
--
-- 0001 created `newsletters` with the columns Ghost had at the time. Ghost 6
-- added typography and shape settings on top, and the email template reads
-- all of them, so the table needs the rest of the set. Names, types and
-- defaults are copied from `core/server/data/schema/schema.js` so a real
-- Ghost database still imports cleanly.
--
-- SQLite's ALTER TABLE ADD COLUMN is cheap and does not rewrite the table.
-- There is no IF NOT EXISTS for it, which is fine: sqlx records the migration
-- and never reruns it.

-- Typography. `title_*` covers the post title and headings; `body_*` the
-- post body.
ALTER TABLE newsletters ADD COLUMN title_font_category TEXT(191) NOT NULL DEFAULT 'sans_serif';
ALTER TABLE newsletters ADD COLUMN title_alignment TEXT(191) NOT NULL DEFAULT 'center';
ALTER TABLE newsletters ADD COLUMN title_font_weight TEXT(50) NOT NULL DEFAULT 'bold';
ALTER TABLE newsletters ADD COLUMN body_font_category TEXT(191) NOT NULL DEFAULT 'sans_serif';

-- Links and buttons. `link_color`/`button_color` already exist and hold
-- either the sentinel 'accent' or a '#rrggbb' value; these add the styling
-- that goes with them.
ALTER TABLE newsletters ADD COLUMN link_style TEXT(50) NOT NULL DEFAULT 'underline';
ALTER TABLE newsletters ADD COLUMN button_style TEXT(50) NOT NULL DEFAULT 'fill';
ALTER TABLE newsletters ADD COLUMN button_corners TEXT(50) NOT NULL DEFAULT 'rounded';

-- Image corners: 'square' or 'rounded'.
ALTER TABLE newsletters ADD COLUMN image_corners TEXT(50) NOT NULL DEFAULT 'square';
