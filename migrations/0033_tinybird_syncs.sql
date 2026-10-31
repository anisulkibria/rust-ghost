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
    last_synced_updated_at DATETIME NOT NULL,
    last_synced_id TEXT(24) NOT NULL,
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);
