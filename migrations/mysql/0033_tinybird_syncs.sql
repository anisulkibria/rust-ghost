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
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `table_name` VARCHAR(191) NOT NULL UNIQUE,
    `last_synced_updated_at` VARCHAR(32) NOT NULL,
    `last_synced_id` VARCHAR(24) NOT NULL,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
