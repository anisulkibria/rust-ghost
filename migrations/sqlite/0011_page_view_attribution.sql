-- Page views gain the two dimensions Ghost's native analytics breaks traffic
-- down by: where the visit came from, and which audience it belongs to.
--
-- `referrer` holds the referring host, already reduced to a source
-- ("google.com", "t.co") rather than a full URL — the admin never needs the
-- path, and keeping it out means no query strings land in the table.
-- NULL means the visit arrived with no referrer, which reads as Direct.
--
-- `audience` is 'public', 'free' or 'paid' as of the moment of the view, so a
-- member upgrading later does not rewrite their own history.
ALTER TABLE page_views ADD COLUMN referrer TEXT(191);
ALTER TABLE page_views ADD COLUMN audience TEXT(20) NOT NULL DEFAULT 'public';

CREATE INDEX IF NOT EXISTS page_views_referrer_idx ON page_views (referrer);
