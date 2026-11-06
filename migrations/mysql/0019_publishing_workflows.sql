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
