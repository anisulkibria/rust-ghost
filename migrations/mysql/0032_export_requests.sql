-- Asynchronous site exports.
--
-- `POST /exports` answers 202 and builds the archive afterwards, because a
-- full export with media in it can take minutes and run to gigabytes — long
-- past any sensible request timeout, and far too large to hold in memory
-- while a browser waits. The row is how a caller finds the finished file, and
-- how the requester is told where to collect it.
CREATE TABLE IF NOT EXISTS export_requests (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    -- The component map as submitted, e.g. {"content":true,"media":false}.
    `components` VARCHAR(4000) NOT NULL,
    `requested_by` VARCHAR(191) ,
    -- pending | running | finished | failed
    `status` VARCHAR(50) NOT NULL DEFAULT 'pending',
    -- Name of the archive under content/exports/, once there is one.
    `filename` VARCHAR(191) ,
    `size_bytes` BIGINT ,
    `error` VARCHAR(2000) ,
    `created_at` VARCHAR(32) NOT NULL,
    `started_at` VARCHAR(32) ,
    `finished_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX export_requests_status_index ON export_requests (`status`);
CREATE INDEX export_requests_requested_by_index ON export_requests (`requested_by`);

-- Foreign keys are added after the tables they point at.
-- SQLite resolves a reference lazily and accepts a forward one;
-- MySQL requires the referenced table to exist already.
ALTER TABLE export_requests ADD CONSTRAINT export_requests_requested_by_fk FOREIGN KEY (`requested_by`) REFERENCES users(`id`) ON DELETE SET NULL;
