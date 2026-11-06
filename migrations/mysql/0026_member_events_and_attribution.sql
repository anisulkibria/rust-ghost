-- Member event history, signup attribution, and a location for a page view.
--
-- Of Ghost's eleven member event tables this schema had one,
-- `members_created_events`, and that one recorded only a bare `source`. So
-- the member detail page had no activity feed to show, the `/members/events`
-- feed had almost nothing to merge, and "which post brought this member in"
-- could not be answered at all.
--
-- The columns are Ghost's, so the same questions can be asked of the same
-- names. `attribution_*` is the resource a signup is credited to — a post, a
-- tag, the homepage — while `referrer_*` and `utm_*` are where the visitor
-- came from before that.

ALTER TABLE members_created_events ADD COLUMN `attribution_id` VARCHAR(24);
ALTER TABLE members_created_events ADD COLUMN `attribution_type` VARCHAR(50);
ALTER TABLE members_created_events ADD COLUMN `attribution_url` VARCHAR(2000);
ALTER TABLE members_created_events ADD COLUMN `referrer_source` VARCHAR(191);
ALTER TABLE members_created_events ADD COLUMN `referrer_medium` VARCHAR(191);
ALTER TABLE members_created_events ADD COLUMN `referrer_url` VARCHAR(2000);
ALTER TABLE members_created_events ADD COLUMN `utm_source` VARCHAR(191);
ALTER TABLE members_created_events ADD COLUMN `utm_medium` VARCHAR(191);
ALTER TABLE members_created_events ADD COLUMN `utm_campaign` VARCHAR(191);
ALTER TABLE members_created_events ADD COLUMN `utm_term` VARCHAR(191);
ALTER TABLE members_created_events ADD COLUMN `utm_content` VARCHAR(191);
CREATE INDEX members_created_events_attribution_idx ON members_created_events (`attribution_type`, `attribution_id`);

CREATE TABLE IF NOT EXISTS members_login_events (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `member_id` VARCHAR(191) NOT NULL ,
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX members_login_events_member_idx ON members_login_events (`member_id`, `created_at`);

-- Free → paid and back. `from_status` is null for the first one, which is the
-- signup itself.
CREATE TABLE IF NOT EXISTS members_status_events (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `member_id` VARCHAR(191) NOT NULL ,
    `from_status` VARCHAR(50) ,
    `to_status` VARCHAR(50) NOT NULL,
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX members_status_events_member_idx ON members_status_events (`member_id`, `created_at`);

CREATE TABLE IF NOT EXISTS members_email_change_events (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `member_id` VARCHAR(191) NOT NULL ,
    `from_email` VARCHAR(191) ,
    `to_email` VARCHAR(191) NOT NULL,
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX members_email_change_events_member_idx ON members_email_change_events (`member_id`, `created_at`);

-- Subscribing and unsubscribing from one newsletter. `subscribed` is which
-- way it went, so an unsubscribe is an event rather than an absence.
CREATE TABLE IF NOT EXISTS members_subscribe_events (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `member_id` VARCHAR(191) NOT NULL ,
    `newsletter_id` VARCHAR(24) ,
    `subscribed` SMALLINT NOT NULL DEFAULT 1,
    `source` VARCHAR(50) ,
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX members_subscribe_events_member_idx ON members_subscribe_events (`member_id`, `created_at`);

-- A paid subscription starting, carrying the same attribution as a signup:
-- the question "which post sold this subscription" is the same shape as
-- "which post brought this member in".
CREATE TABLE IF NOT EXISTS members_subscription_created_events (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `member_id` VARCHAR(191) NOT NULL ,
    `subscription_id` VARCHAR(24) ,
    `attribution_id` VARCHAR(24) ,
    `attribution_type` VARCHAR(50) ,
    `attribution_url` VARCHAR(2000) ,
    `referrer_source` VARCHAR(191) ,
    `referrer_medium` VARCHAR(191) ,
    `referrer_url` VARCHAR(2000) ,
    `utm_source` VARCHAR(191) ,
    `utm_medium` VARCHAR(191) ,
    `utm_campaign` VARCHAR(191) ,
    `utm_term` VARCHAR(191) ,
    `utm_content` VARCHAR(191) ,
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX members_subscription_created_events_member_idx ON members_subscription_created_events (`member_id`, `created_at`);
CREATE INDEX members_subscription_created_events_attribution_idx ON members_subscription_created_events (`attribution_type`, `attribution_id`);

-- A paid subscription changing: started, updated, canceled, expired.
-- `mrr_delta` is what the change did to monthly recurring revenue, which is
-- what makes an MRR series addable rather than recomputable.
CREATE TABLE IF NOT EXISTS members_paid_subscription_events (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `member_id` VARCHAR(191) NOT NULL ,
    `type` VARCHAR(50) NOT NULL,
    `subscription_id` VARCHAR(24) ,
    `from_plan` VARCHAR(191) ,
    `to_plan` VARCHAR(191) ,
    `from_status` VARCHAR(50) ,
    `to_status` VARCHAR(50) ,
    `currency` VARCHAR(3) ,
    `source` VARCHAR(50) ,
    `mrr_delta` BIGINT NOT NULL DEFAULT 0,
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX members_paid_subscription_events_member_idx ON members_paid_subscription_events (`member_id`, `created_at`);
CREATE INDEX members_paid_subscription_events_created_idx ON members_paid_subscription_events (`created_at`);

CREATE TABLE IF NOT EXISTS members_payment_events (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `member_id` VARCHAR(191) NOT NULL ,
    `amount` BIGINT NOT NULL DEFAULT 0,
    `currency` VARCHAR(3) ,
    `source` VARCHAR(50) ,
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX members_payment_events_member_idx ON members_payment_events (`member_id`, `created_at`);

-- A tier being added to or removed from a member.
CREATE TABLE IF NOT EXISTS members_product_events (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `member_id` VARCHAR(191) NOT NULL ,
    `product_id` VARCHAR(24) ,
    `action` VARCHAR(50) NOT NULL,
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX members_product_events_member_idx ON members_product_events (`member_id`, `created_at`);

CREATE TABLE IF NOT EXISTS members_cancel_events (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `member_id` VARCHAR(191) NOT NULL ,
    `subscription_id` VARCHAR(24) ,
    `from_plan` VARCHAR(191) ,
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX members_cancel_events_member_idx ON members_cancel_events (`member_id`, `created_at`);

-- Where a page view came from, beyond the referring host already stored:
-- the source and medium it resolves to, the campaign tags on the URL, and
-- the country, which the analytics page has no dimension for without it.
ALTER TABLE page_views ADD COLUMN `referrer_source` VARCHAR(191);
ALTER TABLE page_views ADD COLUMN `referrer_medium` VARCHAR(191);
ALTER TABLE page_views ADD COLUMN `utm_source` VARCHAR(191);
ALTER TABLE page_views ADD COLUMN `utm_medium` VARCHAR(191);
ALTER TABLE page_views ADD COLUMN `utm_campaign` VARCHAR(191);
ALTER TABLE page_views ADD COLUMN `country` VARCHAR(2);
CREATE INDEX page_views_country_idx ON page_views (`country`);
CREATE INDEX page_views_path_idx ON page_views (`path`(191), `created_at`);

-- Foreign keys are added after the tables they point at.
-- SQLite resolves a reference lazily and accepts a forward one;
-- MySQL requires the referenced table to exist already.
ALTER TABLE members_login_events ADD CONSTRAINT members_login_events_member_id_fk FOREIGN KEY (`member_id`) REFERENCES members(`id`) ON DELETE CASCADE;
ALTER TABLE members_status_events ADD CONSTRAINT members_status_events_member_id_fk FOREIGN KEY (`member_id`) REFERENCES members(`id`) ON DELETE CASCADE;
ALTER TABLE members_email_change_events ADD CONSTRAINT members_email_change_events_member_id_fk FOREIGN KEY (`member_id`) REFERENCES members(`id`) ON DELETE CASCADE;
ALTER TABLE members_subscribe_events ADD CONSTRAINT members_subscribe_events_member_id_fk FOREIGN KEY (`member_id`) REFERENCES members(`id`) ON DELETE CASCADE;
ALTER TABLE members_subscription_created_events ADD CONSTRAINT members_subscription_created_events_member_id_fk FOREIGN KEY (`member_id`) REFERENCES members(`id`) ON DELETE CASCADE;
ALTER TABLE members_paid_subscription_events ADD CONSTRAINT members_paid_subscription_events_member_id_fk FOREIGN KEY (`member_id`) REFERENCES members(`id`) ON DELETE CASCADE;
ALTER TABLE members_payment_events ADD CONSTRAINT members_payment_events_member_id_fk FOREIGN KEY (`member_id`) REFERENCES members(`id`) ON DELETE CASCADE;
ALTER TABLE members_product_events ADD CONSTRAINT members_product_events_member_id_fk FOREIGN KEY (`member_id`) REFERENCES members(`id`) ON DELETE CASCADE;
ALTER TABLE members_cancel_events ADD CONSTRAINT members_cancel_events_member_id_fk FOREIGN KEY (`member_id`) REFERENCES members(`id`) ON DELETE CASCADE;
