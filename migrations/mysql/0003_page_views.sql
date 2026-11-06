-- Lightweight page-view analytics (this port's take on Ghost's web analytics).
-- One row per HTML page view; `viewer` is the anonymous cookie id used to
-- approximate unique visitors. Gated on the `web_analytics` setting.
CREATE TABLE IF NOT EXISTS page_views (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `path` VARCHAR(2000) NOT NULL,
    `viewer` VARCHAR(64) NOT NULL,
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX page_views_created_at_idx ON page_views (`created_at`);
CREATE INDEX page_views_viewer_idx ON page_views (`viewer`);
