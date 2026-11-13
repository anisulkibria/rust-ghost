-- Apps and background work: app installations and their manifest history,
-- email analytics jobs, gift reminders, email verification, and the last two
-- page-view UTM columns.
--
-- Consolidated from earlier migrations. Each section below is one of them (or the
-- named part of one), unchanged and in the order it was first applied.

-- ============================================================================
-- parity_workflows  (was the apps and jobs part of 0028_parity_workflows.sql)
-- ============================================================================

CREATE TABLE gift_reminders (
 token TEXT PRIMARY KEY,
 due_at TEXT NOT NULL,
 sent_at TEXT,
 claimed_at TEXT,
 attempts INTEGER NOT NULL DEFAULT 0,
 last_error TEXT
);
CREATE TABLE email_verifications (
 token_hash TEXT PRIMARY KEY,
 resource_type TEXT NOT NULL,
 resource_id TEXT NOT NULL,
 field TEXT NOT NULL,
 value TEXT NOT NULL,
 expires_at TEXT NOT NULL
);
CREATE TABLE app_installations (
 id TEXT PRIMARY KEY,
 manifest TEXT NOT NULL,
 status TEXT NOT NULL DEFAULT 'active',
 created_at TEXT NOT NULL,
 updated_at TEXT NOT NULL
);
CREATE TABLE analytics_jobs (
 id TEXT PRIMARY KEY,
 email_id TEXT NOT NULL REFERENCES emails(id) ON DELETE CASCADE,
 begin_at TEXT NOT NULL,
 end_at TEXT NOT NULL,
 status TEXT NOT NULL DEFAULT 'pending',
 cursor TEXT,
 error TEXT,
 updated_at TEXT NOT NULL
);
ALTER TABLE page_views ADD COLUMN utm_term TEXT;
ALTER TABLE page_views ADD COLUMN utm_content TEXT;
ALTER TABLE app_installations ADD COLUMN app_id TEXT;
ALTER TABLE app_installations ADD COLUMN manifest_url TEXT;
ALTER TABLE app_installations ADD COLUMN digest TEXT;
ALTER TABLE app_installations ADD COLUMN revision INTEGER NOT NULL DEFAULT 0;
CREATE UNIQUE INDEX app_installations_app ON app_installations(app_id) WHERE status!='uninstalled';
CREATE TABLE app_manifest_history (
 id TEXT PRIMARY KEY,
 installation_id TEXT NOT NULL REFERENCES app_installations(id),
 manifest_url TEXT NOT NULL,
 manifest TEXT NOT NULL,
 digest TEXT NOT NULL,
 source TEXT NOT NULL,
 created_at TEXT NOT NULL
);
ALTER TABLE analytics_jobs ADD COLUMN attempts INTEGER NOT NULL DEFAULT 0;
