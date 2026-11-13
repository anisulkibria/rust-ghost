-- Web analytics: page views, their attribution columns, and the Tinybird sync log.
--
-- Consolidated from earlier migrations. Each section below is one of them (or the
-- named part of one), unchanged and in the order it was first applied.

-- ============================================================================
-- page_views  (was 0003_page_views.sql)
-- ============================================================================

-- Lightweight page-view analytics (this port's take on Ghost's web analytics).
-- One row per HTML page view; `viewer` is the anonymous cookie id used to
-- approximate unique visitors. Gated on the `web_analytics` setting.
CREATE TABLE IF NOT EXISTS page_views (
    id TEXT PRIMARY KEY NOT NULL,
    path TEXT(2000) NOT NULL,
    viewer TEXT(64) NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS page_views_created_at_idx ON page_views (created_at);
CREATE INDEX IF NOT EXISTS page_views_viewer_idx ON page_views (viewer);

-- ============================================================================
-- page_view_attribution  (was 0011_page_view_attribution.sql)
-- ============================================================================

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

-- ============================================================================
-- tinybird_syncs  (was 0033_tinybird_syncs.sql)
-- ============================================================================

-- Where the Tinybird sync got to, per table.
--
-- The sync is incremental and resumable: each run picks up from the last row
-- it confirmed delivered rather than re-reading the table. The cursor is a
-- pair because `updated_at` alone is not unique — two rows written in the
-- same second would make the watermark either skip one or resend it forever,
-- so the row id breaks the tie.
--
-- Matches Ghost's `tinybird_syncs` (`core/server/data/schema/schema.js`) so a
-- database moved between the two carries its position with it.
CREATE TABLE IF NOT EXISTS tinybird_syncs (
    id TEXT PRIMARY KEY NOT NULL,
    table_name TEXT(191) NOT NULL UNIQUE,
    last_synced_updated_at TEXT NOT NULL,
    last_synced_id TEXT(24) NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT
);
