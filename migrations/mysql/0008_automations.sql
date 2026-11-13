-- Automations: the workflow tables, their email designs, and open and click tracking.
--
-- Consolidated from earlier migrations. Each section below is one of them (or the
-- named part of one), unchanged and in the order it was first applied.

-- ============================================================================
-- automations  (was 0010_automations.sql)
-- ============================================================================

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
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `status` VARCHAR(50) NOT NULL DEFAULT 'inactive',
    `name` VARCHAR(191) NOT NULL,
    `description` VARCHAR(2000) NOT NULL DEFAULT '',
    -- Ghost's built-in welcome emails claim reserved slugs; a user-made
    -- automation has none, hence nullable rather than derived from the name.
    `slug` VARCHAR(191) UNIQUE,
    `trigger_tier_scope` VARCHAR(50) ,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX automations_status_idx ON automations (`status`);

-- Which paid tiers start a `selected_paid` automation.
CREATE TABLE IF NOT EXISTS automation_trigger_tiers (
    `automation_id` VARCHAR(191) NOT NULL ,
    `product_id` VARCHAR(191) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE UNIQUE INDEX automation_trigger_tiers_unique ON automation_trigger_tiers (`automation_id`, `product_id`);

CREATE TABLE IF NOT EXISTS automation_actions (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32) NOT NULL,
    -- Soft delete: a finished run still references the revisions of a node
    -- the author has since removed from the chain.
    `deleted_at` VARCHAR(32) ,
    `automation_id` VARCHAR(191) NOT NULL ,
    `type` VARCHAR(50) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX automation_actions_automation_idx ON automation_actions (`automation_id`, `deleted_at`);

CREATE TABLE IF NOT EXISTS automation_action_revisions (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `created_at` VARCHAR(32) NOT NULL,
    `action_id` VARCHAR(191) NOT NULL ,
    `wait_hours` BIGINT UNSIGNED,
    `email_subject` VARCHAR(300) ,
    `email_lexical` MEDIUMTEXT ,
    -- Ghost points this at its `email_design_settings` table. This port keeps
    -- email design on `newsletters`, so the column is carried for import
    -- fidelity and left null; the sender resolves design from the newsletter.
    `email_design_setting_id` VARCHAR(24) ,
    `email_sent_count` BIGINT UNSIGNED,
    `email_opened_count` BIGINT UNSIGNED,
    `email_clicked_count` BIGINT UNSIGNED
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
-- Ghost's uniqueness constraint. It is also what makes "the latest revision"
-- well defined: two revisions of one action cannot share a timestamp.
CREATE UNIQUE INDEX automation_action_revisions_unique ON automation_action_revisions (`created_at`, `action_id`);
CREATE INDEX automation_action_revisions_action_idx ON automation_action_revisions (`action_id`, `created_at`);

CREATE TABLE IF NOT EXISTS automation_action_edges (
    `source_action_id` VARCHAR(191) NOT NULL ,
    `target_action_id` VARCHAR(191) NOT NULL ,
    PRIMARY KEY (source_action_id, target_action_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX automation_action_edges_target_idx ON automation_action_edges (`target_action_id`);

-- One member's journey through one automation.
CREATE TABLE IF NOT EXISTS automation_runs (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32) NOT NULL,
    `automation_id` VARCHAR(191) NOT NULL ,
    -- Null once the member is deleted; the run stays as history.
    `member_id` VARCHAR(191) ,
    -- The address at entry. Historical record only — a send always re-reads
    -- the member's current address.
    `member_email` VARCHAR(191) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX automation_runs_automation_idx ON automation_runs (`automation_id`, `created_at`);
CREATE INDEX automation_runs_member_idx ON automation_runs (`member_id`);
CREATE INDEX automation_runs_updated_idx ON automation_runs (`updated_at`);

CREATE TABLE IF NOT EXISTS automation_run_steps (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32) NOT NULL,
    `automation_run_id` VARCHAR(191) NOT NULL ,
    `automation_action_revision_id` VARCHAR(191) NOT NULL ,
    `ready_at` VARCHAR(32) NOT NULL,
    `step_attempts` BIGINT UNSIGNED NOT NULL DEFAULT 0,
    `started_at` VARCHAR(32) ,
    `finished_at` VARCHAR(32) ,
    -- pending | finished | failed | automation disabled
    --         | member changed status | member unsubscribed
    `status` VARCHAR(50) NOT NULL DEFAULT 'pending',
    -- Claimed by one worker for the duration of an attempt. A lock older than
    -- the timeout is treated as abandoned.
    `locked_by` VARCHAR(191) ,
    `locked_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX automation_run_steps_ready_idx ON automation_run_steps (`status`, `ready_at`, `created_at`, `id`);
CREATE INDEX automation_run_steps_run_idx ON automation_run_steps (`automation_run_id`, `created_at`);
CREATE INDEX automation_run_steps_updated_idx ON automation_run_steps (`updated_at`);

-- One row per automation email actually sent, which is where the per-action
-- sent/opened/clicked counts come from.
CREATE TABLE IF NOT EXISTS automated_email_recipients (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `automated_email_id` VARCHAR(24) ,
    `automation_action_revision_id` VARCHAR(191) ,
    `automation_run_step_id` VARCHAR(191) ,
    `member_id` VARCHAR(24) NOT NULL,
    `member_uuid` VARCHAR(36) NOT NULL,
    `member_email` VARCHAR(191) NOT NULL,
    `member_name` VARCHAR(191) ,
    `mailgun_message_id` VARCHAR(1000) ,
    `delivered_at` VARCHAR(32) ,
    `opened_at` VARCHAR(32) ,
    `clicked_at` VARCHAR(32) ,
    `track_opens` SMALLINT NOT NULL DEFAULT 0,
    `track_clicks` SMALLINT NOT NULL DEFAULT 0,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX automated_email_recipients_member_idx ON automated_email_recipients (`member_id`);
CREATE INDEX automated_email_recipients_revision_idx ON automated_email_recipients (`automation_action_revision_id`);

-- Foreign keys are added after the tables they point at.
-- SQLite resolves a reference lazily and accepts a forward one;
-- MySQL requires the referenced table to exist already.
ALTER TABLE automation_trigger_tiers ADD CONSTRAINT automation_trigger_tiers_automation_id_fk FOREIGN KEY (`automation_id`) REFERENCES automations(`id`) ON DELETE CASCADE;
ALTER TABLE automation_trigger_tiers ADD CONSTRAINT automation_trigger_tiers_product_id_fk2 FOREIGN KEY (`product_id`) REFERENCES products(`id`);
ALTER TABLE automation_actions ADD CONSTRAINT automation_actions_automation_id_fk FOREIGN KEY (`automation_id`) REFERENCES automations(`id`);
ALTER TABLE automation_action_revisions ADD CONSTRAINT automation_action_revisions_action_id_fk FOREIGN KEY (`action_id`) REFERENCES automation_actions(`id`);
ALTER TABLE automation_action_edges ADD CONSTRAINT automation_action_edges_source_action_id_fk FOREIGN KEY (`source_action_id`) REFERENCES automation_actions(`id`);
ALTER TABLE automation_action_edges ADD CONSTRAINT automation_action_edges_target_action_id_fk2 FOREIGN KEY (`target_action_id`) REFERENCES automation_actions(`id`);
ALTER TABLE automation_runs ADD CONSTRAINT automation_runs_automation_id_fk FOREIGN KEY (`automation_id`) REFERENCES automations(`id`);
ALTER TABLE automation_runs ADD CONSTRAINT automation_runs_member_id_fk2 FOREIGN KEY (`member_id`) REFERENCES members(`id`) ON DELETE SET NULL;
ALTER TABLE automation_run_steps ADD CONSTRAINT automation_run_steps_automation_run_id_fk FOREIGN KEY (`automation_run_id`) REFERENCES automation_runs(`id`);
ALTER TABLE automation_run_steps ADD CONSTRAINT automation_run_steps_automation_action_revision_id_fk2 FOREIGN KEY (`automation_action_revision_id`) REFERENCES automation_action_revisions(`id`);
ALTER TABLE automated_email_recipients ADD CONSTRAINT automated_email_recipients_automation_action_revision_id_fk FOREIGN KEY (`automation_action_revision_id`) REFERENCES automation_action_revisions(`id`);
ALTER TABLE automated_email_recipients ADD CONSTRAINT automated_email_recipients_automation_run_step_id_fk2 FOREIGN KEY (`automation_run_step_id`) REFERENCES automation_run_steps(`id`);

-- ============================================================================
-- automation_email_designs  (was 0022_automation_email_designs.sql)
-- ============================================================================

CREATE TABLE email_design_settings (
    `id` VARCHAR(191) PRIMARY KEY,
    `settings` VARCHAR(4000) NOT NULL,
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- ============================================================================
-- parity_workflows  (was the automation tracking part of 0028_parity_workflows.sql)
-- ============================================================================

CREATE TABLE automation_tracking (
 `token` VARCHAR(191) PRIMARY KEY,
 `revision_id` VARCHAR(191) NOT NULL ,
 `member_uuid` VARCHAR(191) NOT NULL,
 `send_id` VARCHAR(191) NOT NULL DEFAULT '',
 `opened_at` VARCHAR(32) ,
 `clicked_at` VARCHAR(32) ,
 UNIQUE(revision_id,member_uuid,send_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE TABLE automation_links (
 `id` VARCHAR(191) PRIMARY KEY,
 `revision_id` VARCHAR(191) NOT NULL ,
 `url` VARCHAR(191) NOT NULL,
 UNIQUE(revision_id,url)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE TABLE automation_link_clicks (
 `link_id` VARCHAR(191) NOT NULL ,
 `token` VARCHAR(191) NOT NULL ,
 `created_at` VARCHAR(32) NOT NULL,
 PRIMARY KEY(link_id,token)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Foreign keys are added after the tables they point at.
-- SQLite resolves a reference lazily and accepts a forward one;
-- MySQL requires the referenced table to exist already.
ALTER TABLE automation_tracking ADD CONSTRAINT automation_tracking_revision_id_fk FOREIGN KEY (`revision_id`) REFERENCES automation_action_revisions(`id`);
ALTER TABLE automation_links ADD CONSTRAINT automation_links_revision_id_fk FOREIGN KEY (`revision_id`) REFERENCES automation_action_revisions(`id`);
ALTER TABLE automation_link_clicks ADD CONSTRAINT automation_link_clicks_link_id_fk FOREIGN KEY (`link_id`) REFERENCES automation_links(`id`);
ALTER TABLE automation_link_clicks ADD CONSTRAINT automation_link_clicks_token_fk2 FOREIGN KEY (`token`) REFERENCES automation_tracking(`token`);
