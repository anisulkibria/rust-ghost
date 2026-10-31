-- Asynchronous site exports.
--
-- `POST /exports` answers 202 and builds the archive afterwards, because a
-- full export with media in it can take minutes and run to gigabytes — long
-- past any sensible request timeout, and far too large to hold in memory
-- while a browser waits. The row is how a caller finds the finished file, and
-- how the requester is told where to collect it.
CREATE TABLE IF NOT EXISTS export_requests (
    id TEXT PRIMARY KEY NOT NULL,
    -- The component map as submitted, e.g. {"content":true,"media":false}.
    components TEXT NOT NULL,
    requested_by TEXT(24) REFERENCES users(id) ON DELETE SET NULL,
    -- pending | running | finished | failed
    status TEXT(50) NOT NULL DEFAULT 'pending',
    -- Name of the archive under content/exports/, once there is one.
    filename TEXT(191),
    size_bytes BIGINT,
    error TEXT(2000),
    created_at DATETIME NOT NULL,
    started_at DATETIME,
    finished_at DATETIME
);

CREATE INDEX IF NOT EXISTS export_requests_status_index ON export_requests (status);
CREATE INDEX IF NOT EXISTS export_requests_requested_by_index ON export_requests (requested_by);
