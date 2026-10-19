-- Lightweight page-view analytics (this port's take on Ghost's web analytics).
-- One row per HTML page view; `viewer` is the anonymous cookie id used to
-- approximate unique visitors. Gated on the `web_analytics` setting.
CREATE TABLE IF NOT EXISTS page_views (
    id TEXT PRIMARY KEY NOT NULL,
    path TEXT(2000) NOT NULL,
    viewer TEXT(64) NOT NULL,
    created_at DATETIME NOT NULL
);
CREATE INDEX IF NOT EXISTS page_views_created_at_idx ON page_views (created_at);
CREATE INDEX IF NOT EXISTS page_views_viewer_idx ON page_views (viewer);
