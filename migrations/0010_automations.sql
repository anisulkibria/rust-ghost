-- Automations: a chain of wait/send-email steps a member walks after signup.
--
-- Column names, types and nullability follow Ghost 6.72's
-- `core/server/data/schema/schema.js` so a Ghost export lands here unchanged.
-- Two shapes in that schema are worth knowing before reading the rest:
--
-- * **Actions are versioned.** `automation_actions` is the stable identity of
--   a node; `automation_action_revisions` holds its contents at a point in
--   time. Editing an automation writes a new revision rather than updating
--   the old one, and a run in flight keeps pointing at the revision it
--   started with — so changing an email tomorrow does not rewrite what was
--   sent today. Deleting a node is a soft delete for the same reason.
--
-- * **The chain is stored as a graph.** `automation_action_edges` holds
--   source→target pairs rather than a sort order. Ghost only accepts a single
--   linear path today, but the storage is already shaped for branches, and
--   matching it means a future Ghost automation imports without a rewrite.

CREATE TABLE IF NOT EXISTS automations (
    id TEXT PRIMARY KEY NOT NULL,
    status TEXT(50) NOT NULL DEFAULT 'inactive',
    name TEXT(191) NOT NULL,
    description TEXT(2000) NOT NULL DEFAULT '',
    -- Ghost's built-in welcome emails claim reserved slugs; a user-made
    -- automation has none, hence nullable rather than derived from the name.
    slug TEXT(191) UNIQUE,
    trigger_tier_scope TEXT(50),
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);
CREATE INDEX IF NOT EXISTS automations_status_idx ON automations (status);

-- Which paid tiers start a `selected_paid` automation.
CREATE TABLE IF NOT EXISTS automation_trigger_tiers (
    automation_id TEXT(24) NOT NULL REFERENCES automations(id) ON DELETE CASCADE,
    product_id TEXT(24) NOT NULL REFERENCES products(id)
);
CREATE UNIQUE INDEX IF NOT EXISTS automation_trigger_tiers_unique
    ON automation_trigger_tiers (automation_id, product_id);

CREATE TABLE IF NOT EXISTS automation_actions (
    id TEXT PRIMARY KEY NOT NULL,
    created_at DATETIME NOT NULL,
    updated_at DATETIME NOT NULL,
    -- Soft delete: a finished run still references the revisions of a node
    -- the author has since removed from the chain.
    deleted_at DATETIME,
    automation_id TEXT(24) NOT NULL REFERENCES automations(id),
    type TEXT(50) NOT NULL
);
CREATE INDEX IF NOT EXISTS automation_actions_automation_idx
    ON automation_actions (automation_id, deleted_at);

CREATE TABLE IF NOT EXISTS automation_action_revisions (
    id TEXT PRIMARY KEY NOT NULL,
    created_at DATETIME NOT NULL,
    action_id TEXT(24) NOT NULL REFERENCES automation_actions(id),
    wait_hours INTEGER UNSIGNED,
    email_subject TEXT(300),
    email_lexical TEXT,
    -- Ghost points this at its `email_design_settings` table. This port keeps
    -- email design on `newsletters`, so the column is carried for import
    -- fidelity and left null; the sender resolves design from the newsletter.
    email_design_setting_id TEXT(24),
    email_sent_count INTEGER UNSIGNED,
    email_opened_count INTEGER UNSIGNED,
    email_clicked_count INTEGER UNSIGNED
);
-- Ghost's uniqueness constraint. It is also what makes "the latest revision"
-- well defined: two revisions of one action cannot share a timestamp.
CREATE UNIQUE INDEX IF NOT EXISTS automation_action_revisions_unique
    ON automation_action_revisions (created_at, action_id);
CREATE INDEX IF NOT EXISTS automation_action_revisions_action_idx
    ON automation_action_revisions (action_id, created_at);

CREATE TABLE IF NOT EXISTS automation_action_edges (
    source_action_id TEXT(24) NOT NULL REFERENCES automation_actions(id),
    target_action_id TEXT(24) NOT NULL REFERENCES automation_actions(id),
    PRIMARY KEY (source_action_id, target_action_id)
);
CREATE INDEX IF NOT EXISTS automation_action_edges_target_idx
    ON automation_action_edges (target_action_id);

-- One member's journey through one automation.
CREATE TABLE IF NOT EXISTS automation_runs (
    id TEXT PRIMARY KEY NOT NULL,
    created_at DATETIME NOT NULL,
    updated_at DATETIME NOT NULL,
    automation_id TEXT(24) NOT NULL REFERENCES automations(id),
    -- Null once the member is deleted; the run stays as history.
    member_id TEXT(24) REFERENCES members(id) ON DELETE SET NULL,
    -- The address at entry. Historical record only — a send always re-reads
    -- the member's current address.
    member_email TEXT(191) NOT NULL
);
CREATE INDEX IF NOT EXISTS automation_runs_automation_idx ON automation_runs (automation_id, created_at);
CREATE INDEX IF NOT EXISTS automation_runs_member_idx ON automation_runs (member_id);
CREATE INDEX IF NOT EXISTS automation_runs_updated_idx ON automation_runs (updated_at);

CREATE TABLE IF NOT EXISTS automation_run_steps (
    id TEXT PRIMARY KEY NOT NULL,
    created_at DATETIME NOT NULL,
    updated_at DATETIME NOT NULL,
    automation_run_id TEXT(24) NOT NULL REFERENCES automation_runs(id),
    automation_action_revision_id TEXT(24) NOT NULL REFERENCES automation_action_revisions(id),
    ready_at DATETIME NOT NULL,
    step_attempts INTEGER UNSIGNED NOT NULL DEFAULT 0,
    started_at DATETIME,
    finished_at DATETIME,
    -- pending | finished | failed | automation disabled
    --         | member changed status | member unsubscribed
    status TEXT(50) NOT NULL DEFAULT 'pending',
    -- Claimed by one worker for the duration of an attempt. A lock older than
    -- the timeout is treated as abandoned.
    locked_by TEXT(191),
    locked_at DATETIME
);
CREATE INDEX IF NOT EXISTS automation_run_steps_ready_idx
    ON automation_run_steps (status, ready_at, created_at, id);
CREATE INDEX IF NOT EXISTS automation_run_steps_run_idx
    ON automation_run_steps (automation_run_id, created_at);
CREATE INDEX IF NOT EXISTS automation_run_steps_updated_idx
    ON automation_run_steps (updated_at);

-- One row per automation email actually sent, which is where the per-action
-- sent/opened/clicked counts come from.
CREATE TABLE IF NOT EXISTS automated_email_recipients (
    id TEXT PRIMARY KEY NOT NULL,
    automated_email_id TEXT(24),
    automation_action_revision_id TEXT(24) REFERENCES automation_action_revisions(id),
    automation_run_step_id TEXT(24) REFERENCES automation_run_steps(id),
    member_id TEXT(24) NOT NULL,
    member_uuid TEXT(36) NOT NULL,
    member_email TEXT(191) NOT NULL,
    member_name TEXT(191),
    mailgun_message_id TEXT(1000),
    delivered_at DATETIME,
    opened_at DATETIME,
    clicked_at DATETIME,
    track_opens BOOLEAN NOT NULL DEFAULT 0,
    track_clicks BOOLEAN NOT NULL DEFAULT 0,
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);
CREATE INDEX IF NOT EXISTS automated_email_recipients_member_idx
    ON automated_email_recipients (member_id);
CREATE INDEX IF NOT EXISTS automated_email_recipients_revision_idx
    ON automated_email_recipients (automation_action_revision_id);
