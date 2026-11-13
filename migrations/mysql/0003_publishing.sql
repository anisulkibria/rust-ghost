-- Site and publishing: the editor's markdown source, staff invites, the site name,
-- publishing workflows, gift links, recommendations, exports and integration roles.
--
-- Consolidated from earlier migrations. Each section below is one of them (or the
-- named part of one), unchanged and in the order it was first applied.

-- ============================================================================
-- editor_markdown  (was 0002_editor_markdown.sql)
-- ============================================================================

-- Markdown source for posts authored in the built-in editor (round-trip).
-- Ghost itself stores editor state in mobiledoc/lexical; ghost-rust's admin
-- editor is markdown-first so it keeps the source here. Extra columns don't
-- affect INSERT INTO ... SELECT imports from a real Ghost database.
ALTER TABLE posts ADD COLUMN `markdown` MEDIUMTEXT;

-- ============================================================================
-- invites_email_status  (was 0006_invites_email_status.sql)
-- ============================================================================

-- Staff invites need an email and a status.
--
-- The initial schema carried only `role_id`, `token` and `expires`, which is
-- enough to *redeem* an invite but not to show one: Settings → Staff lists
-- pending invitations by address, and resending one has to know where to send
-- it. Ghost's own `invites` table has both columns (status is
-- 'pending' | 'sent'), so this brings the table in line.
--
-- SQLite's ALTER TABLE ADD COLUMN cannot add a UNIQUE column, so uniqueness
-- on email is enforced by a separate index — which is what we want anyway:
-- one outstanding invite per address.
ALTER TABLE invites ADD COLUMN `email` VARCHAR(191) NOT NULL DEFAULT '';
ALTER TABLE invites ADD COLUMN `status` VARCHAR(50) NOT NULL DEFAULT 'pending';

CREATE UNIQUE INDEX invites_email_unique ON invites (`email`);

-- ============================================================================
-- site_name  (was 0018_site_name.sql)
-- ============================================================================

-- Keep the short publication name separate from its title. A blank name
-- falls back to the current title in themes and the admin UI.
INSERT INTO settings (id, `group`, `key`, value, type, created_at, updated_at)
SELECT lower(hex(RANDOM_BYTES(12))), 'site', 'name', '', 'string', UTC_TIMESTAMP(), UTC_TIMESTAMP()
WHERE NOT EXISTS (SELECT 1 FROM settings WHERE `key` = 'name');

-- ============================================================================
-- publishing_workflows  (was 0019_publishing_workflows.sql)
-- ============================================================================

-- Persistent workflows shared by the admin UI, public endpoints and APIs.
ALTER TABLE post_revisions ADD COLUMN `snapshot` MEDIUMTEXT;
CREATE TABLE email_suppressions (
    `email` VARCHAR(191) PRIMARY KEY,
    `reason` VARCHAR(1000) NOT NULL,
    `provider` VARCHAR(191) NOT NULL DEFAULT 'local',
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE TABLE email_delivery_events (
    `id` VARCHAR(191) PRIMARY KEY,
    `event_type` VARCHAR(1000) NOT NULL,
    `email` VARCHAR(1000) NOT NULL,
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE TABLE newsletter_links (
    `id` VARCHAR(191) PRIMARY KEY,
    `post_id` VARCHAR(191) NOT NULL ,
    `from_url` VARCHAR(191) NOT NULL,
    `to_url` VARCHAR(1000) NOT NULL,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32) NOT NULL,
    UNIQUE(post_id, from_url)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE TABLE newsletter_clicks (
    `id` VARCHAR(191) PRIMARY KEY,
    `link_id` VARCHAR(191) NOT NULL ,
    `member_uuid` VARCHAR(1000) NOT NULL,
    `created_at` VARCHAR(191) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX newsletter_clicks_link ON newsletter_clicks (`link_id`, `created_at`);
CREATE TABLE saved_views (
    `id` VARCHAR(191) PRIMARY KEY,
    `user_id` VARCHAR(191) NOT NULL ,
    `resource` VARCHAR(191) NOT NULL,
    `name` VARCHAR(191) NOT NULL,
    `filter` VARCHAR(4000) NOT NULL,
    `created_at` VARCHAR(32) NOT NULL,
    UNIQUE(user_id, resource, name)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE TABLE article_gift_links (
    `id` VARCHAR(191) PRIMARY KEY,
    `post_id` VARCHAR(191) NOT NULL UNIQUE ,
    `token_hash` VARCHAR(191) NOT NULL UNIQUE,
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE TABLE staff_password_resets (
    `token_hash` VARCHAR(191) PRIMARY KEY,
    `user_id` VARCHAR(191) NOT NULL ,
    `expires_at` VARCHAR(32) NOT NULL,
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE TABLE scheduled_gift_deliveries (
    `token` VARCHAR(191) PRIMARY KEY,
    `deliver_at` VARCHAR(32) NOT NULL,
    `sent_at` VARCHAR(32) ,
    `attempts` BIGINT NOT NULL DEFAULT 0,
    `last_error` VARCHAR(1000)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
ALTER TABLE labels ADD COLUMN `slug` VARCHAR(1000);
UPDATE labels SET slug=lower(replace(name,' ','-')) WHERE slug IS NULL;
CREATE TRIGGER labels_slug_on_insert BEFORE INSERT ON labels FOR EACH ROW
SET NEW.slug = IFNULL(NEW.slug, LOWER(REPLACE(NEW.name, ' ', '-')));
ALTER TABLE page_views ADD COLUMN `device` VARCHAR(191) NOT NULL DEFAULT 'unknown';
ALTER TABLE page_views ADD COLUMN `browser` VARCHAR(191) NOT NULL DEFAULT 'unknown';
CREATE INDEX page_views_realtime ON page_views (`created_at`, `viewer`);
ALTER TABLE scheduled_gift_deliveries ADD COLUMN `last_attempt_at` VARCHAR(32);
ALTER TABLE posts ADD COLUMN `newsletter_audience_filter` MEDIUMTEXT;
ALTER TABLE posts ADD COLUMN `newsletter_include_preview` SMALLINT NOT NULL DEFAULT 0;
ALTER TABLE members_products ADD COLUMN `subscription_id` VARCHAR(191);
CREATE INDEX members_products_subscription ON members_products (`subscription_id`);
-- Existing recurring grants can be attributed where their price mapping is known.
UPDATE members_products SET subscription_id=(
    SELECT s.subscription_id FROM subscriptions s
    JOIN members_stripe_customers c ON c.customer_id=s.customer_id
    JOIN products p ON p.id=members_products.product_id
    WHERE c.member_id=members_products.member_id AND s.status IN ('active','trialing')
      AND (p.monthly_price_id=s.stripe_price_id OR p.yearly_price_id=s.stripe_price_id)
    ORDER BY s.created_at DESC LIMIT 1
) WHERE expiry_at IS NULL;
INSERT IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at)
VALUES (lower(hex(RANDOM_BYTES(12))),'email','mailgun_webhook_signing_key','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP());

-- Foreign keys are added after the tables they point at.
-- SQLite resolves a reference lazily and accepts a forward one;
-- MySQL requires the referenced table to exist already.
ALTER TABLE newsletter_links ADD CONSTRAINT newsletter_links_post_id_fk FOREIGN KEY (`post_id`) REFERENCES posts(`id`) ON DELETE CASCADE;
ALTER TABLE newsletter_clicks ADD CONSTRAINT newsletter_clicks_link_id_fk FOREIGN KEY (`link_id`) REFERENCES newsletter_links(`id`) ON DELETE CASCADE;
ALTER TABLE saved_views ADD CONSTRAINT saved_views_user_id_fk FOREIGN KEY (`user_id`) REFERENCES users(`id`) ON DELETE CASCADE;
ALTER TABLE article_gift_links ADD CONSTRAINT article_gift_links_post_id_fk FOREIGN KEY (`post_id`) REFERENCES posts(`id`) ON DELETE CASCADE;
ALTER TABLE staff_password_resets ADD CONSTRAINT staff_password_resets_user_id_fk FOREIGN KEY (`user_id`) REFERENCES users(`id`) ON DELETE CASCADE;

-- ============================================================================
-- article_gift_history  (was 0030_article_gift_history.sql)
-- ============================================================================

-- The live association is separate from historical tokens, as in Ghost 6.73.
-- Legacy hashed links remain valid until their post's next reset/revocation.
CREATE TABLE gift_links (
    `token` VARCHAR(191) NOT NULL PRIMARY KEY,
    `post_id` VARCHAR(191) NOT NULL ,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX gift_links_post_id ON gift_links (`post_id`);
CREATE TABLE post_gift_links (
    `post_id` VARCHAR(191) NOT NULL UNIQUE ,
    `gift_link_token` VARCHAR(191) NOT NULL PRIMARY KEY ,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Foreign keys are added after the tables they point at.
-- SQLite resolves a reference lazily and accepts a forward one;
-- MySQL requires the referenced table to exist already.
ALTER TABLE gift_links ADD CONSTRAINT gift_links_post_id_fk FOREIGN KEY (`post_id`) REFERENCES posts(`id`) ON DELETE CASCADE;
ALTER TABLE post_gift_links ADD CONSTRAINT post_gift_links_post_id_fk FOREIGN KEY (`post_id`) REFERENCES posts(`id`) ON DELETE CASCADE;
ALTER TABLE post_gift_links ADD CONSTRAINT post_gift_links_gift_link_token_fk2 FOREIGN KEY (`gift_link_token`) REFERENCES gift_links(`token`) ON DELETE CASCADE;

-- ============================================================================
-- recommendation_metadata  (was 0031_recommendation_metadata.sql)
-- ============================================================================

-- Recommendation display metadata.
--
-- `POST /recommendations/check` already reads a candidate publication's
-- favicon, social image and excerpt out of its HTML, but there was nowhere to
-- keep them, so every saved recommendation rendered as a bare title and link.
-- Ghost stores all three on the row (`core/server/data/schema/schema.js`,
-- `recommendations`) and its theme helper and Portal list both read them.
ALTER TABLE recommendations ADD COLUMN `excerpt` VARCHAR(2000);
ALTER TABLE recommendations ADD COLUMN `featured_image` VARCHAR(2000);
ALTER TABLE recommendations ADD COLUMN `favicon` VARCHAR(2000);

-- ============================================================================
-- export_requests  (was 0032_export_requests.sql)
-- ============================================================================

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

-- ============================================================================
-- internal_integration_roles  (was 0034_internal_integration_roles.sql)
-- ============================================================================

-- Least privilege for the two internal integrations.
--
-- Both were seeded with the broad "Admin Integration" role, which grants
-- almost everything: posts, members, settings, themes, users, webhooks. The
-- scheduler needs to publish a post. The backup needs to read and export.
-- Neither needs the rest, and both keys are readable from the database and
-- visible on the integrations screen — so the difference is the blast radius
-- if one gets out.
--
-- These are Ghost's own roles for these integrations
-- (`services/permissions/permissions-data.js`). Matched here so a key that
-- leaks can do only what its integration was ever for.
--
-- Scoped by integration slug, so a custom integration an operator
-- deliberately gave the admin role keeps it.
UPDATE api_keys
SET role_id = (SELECT id FROM roles WHERE name = 'Scheduler Integration'),
    updated_at = UTC_TIMESTAMP()
WHERE type = 'admin'
  AND integration_id IN (SELECT id FROM integrations WHERE slug = 'ghost-scheduler')
  AND EXISTS (SELECT 1 FROM roles WHERE name = 'Scheduler Integration');

UPDATE api_keys
SET role_id = (SELECT id FROM roles WHERE name = 'DB Backup Integration'),
    updated_at = UTC_TIMESTAMP()
WHERE type = 'admin'
  AND integration_id IN (SELECT id FROM integrations WHERE slug = 'ghost-backup')
  AND EXISTS (SELECT 1 FROM roles WHERE name = 'DB Backup Integration');
