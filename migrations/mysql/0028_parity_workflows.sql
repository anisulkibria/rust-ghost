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
CREATE TABLE gift_reminders (
 `token` VARCHAR(191) PRIMARY KEY,
 `due_at` VARCHAR(32) NOT NULL,
 `sent_at` VARCHAR(32) ,
 `claimed_at` VARCHAR(32) ,
 `attempts` BIGINT NOT NULL DEFAULT 0,
 `last_error` VARCHAR(1000)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE TABLE email_verifications (
 `token_hash` VARCHAR(191) PRIMARY KEY,
 `resource_type` VARCHAR(1000) NOT NULL,
 `resource_id` VARCHAR(1000) NOT NULL,
 `field` VARCHAR(1000) NOT NULL,
 `value` VARCHAR(4000) NOT NULL,
 `expires_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE TABLE app_installations (
 `id` VARCHAR(191) PRIMARY KEY,
 `manifest` VARCHAR(4000) NOT NULL,
 `status` VARCHAR(191) NOT NULL DEFAULT 'active',
 `created_at` VARCHAR(32) NOT NULL,
 `updated_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE TABLE analytics_jobs (
 `id` VARCHAR(191) PRIMARY KEY,
 `email_id` VARCHAR(191) NOT NULL ,
 `begin_at` VARCHAR(32) NOT NULL,
 `end_at` VARCHAR(32) NOT NULL,
 `status` VARCHAR(191) NOT NULL DEFAULT 'pending',
 `cursor` VARCHAR(1000) ,
 `error` VARCHAR(1000) ,
 `updated_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
ALTER TABLE page_views ADD COLUMN `utm_term` VARCHAR(1000);
ALTER TABLE page_views ADD COLUMN `utm_content` VARCHAR(1000);
ALTER TABLE app_installations ADD COLUMN `app_id` VARCHAR(191);
ALTER TABLE app_installations ADD COLUMN `manifest_url` VARCHAR(1000);
ALTER TABLE app_installations ADD COLUMN `digest` VARCHAR(1000);
ALTER TABLE app_installations ADD COLUMN `revision` BIGINT NOT NULL DEFAULT 0;
-- MySQL has no partial index. This generated column is NULL
-- unless the condition holds, and NULLs do not collide in a
-- unique index -- which is exactly what `WHERE status!='uninstalled'` means.
ALTER TABLE app_installations ADD COLUMN `app_installations_app_active` VARCHAR(191) GENERATED ALWAYS AS (IF(status!='uninstalled', `app_id`, NULL)) VIRTUAL;
CREATE UNIQUE INDEX app_installations_app ON app_installations (`app_installations_app_active`);
CREATE TABLE app_manifest_history (
 `id` VARCHAR(191) PRIMARY KEY,
 `installation_id` VARCHAR(191) NOT NULL ,
 `manifest_url` VARCHAR(1000) NOT NULL,
 `manifest` VARCHAR(4000) NOT NULL,
 `digest` VARCHAR(1000) NOT NULL,
 `source` VARCHAR(4000) NOT NULL,
 `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
ALTER TABLE analytics_jobs ADD COLUMN `attempts` BIGINT NOT NULL DEFAULT 0;

-- Foreign keys are added after the tables they point at.
-- SQLite resolves a reference lazily and accepts a forward one;
-- MySQL requires the referenced table to exist already.
ALTER TABLE automation_tracking ADD CONSTRAINT automation_tracking_revision_id_fk FOREIGN KEY (`revision_id`) REFERENCES automation_action_revisions(`id`);
ALTER TABLE automation_links ADD CONSTRAINT automation_links_revision_id_fk FOREIGN KEY (`revision_id`) REFERENCES automation_action_revisions(`id`);
ALTER TABLE automation_link_clicks ADD CONSTRAINT automation_link_clicks_link_id_fk FOREIGN KEY (`link_id`) REFERENCES automation_links(`id`);
ALTER TABLE automation_link_clicks ADD CONSTRAINT automation_link_clicks_token_fk2 FOREIGN KEY (`token`) REFERENCES automation_tracking(`token`);
ALTER TABLE analytics_jobs ADD CONSTRAINT analytics_jobs_email_id_fk FOREIGN KEY (`email_id`) REFERENCES emails(`id`) ON DELETE CASCADE;
ALTER TABLE app_manifest_history ADD CONSTRAINT app_manifest_history_installation_id_fk FOREIGN KEY (`installation_id`) REFERENCES app_installations(`id`);
