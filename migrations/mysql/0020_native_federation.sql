CREATE TABLE federation_keys (
    `id` VARCHAR(191) PRIMARY KEY,
    `private_pem` VARCHAR(4000) NOT NULL,
    `public_pem` VARCHAR(4000) NOT NULL,
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE TABLE federation_followers (
    `actor` VARCHAR(191) PRIMARY KEY,
    `inbox` VARCHAR(1000) NOT NULL,
    `follow_id` VARCHAR(1000) NOT NULL,
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE TABLE federation_following (
    `actor` VARCHAR(191) PRIMARY KEY,
    `inbox` VARCHAR(1000) NOT NULL,
    `follow_id` VARCHAR(1000) NOT NULL,
    `accepted` SMALLINT NOT NULL DEFAULT 0,
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE TABLE federation_activities (
    `id` VARCHAR(191) PRIMARY KEY,
    `activity_type` VARCHAR(1000) NOT NULL,
    `object_id` VARCHAR(1000) ,
    `body` VARCHAR(4000) NOT NULL,
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE TABLE federation_received (
    `id` VARCHAR(191) PRIMARY KEY,
    `actor` VARCHAR(1000) NOT NULL,
    `activity_type` VARCHAR(1000) NOT NULL,
    `body` VARCHAR(4000) NOT NULL,
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE TABLE federation_objects (
    `id` VARCHAR(191) PRIMARY KEY,
    `actor` VARCHAR(1000) NOT NULL,
    `object_type` VARCHAR(1000) NOT NULL,
    `content` VARCHAR(4000) NOT NULL,
    `url` VARCHAR(1000) NOT NULL,
    `in_reply_to` VARCHAR(191) ,
    `published_at` VARCHAR(191) NOT NULL,
    `updated_at` VARCHAR(32) NOT NULL,
    `local` SMALLINT NOT NULL DEFAULT 0,
    `deleted` SMALLINT NOT NULL DEFAULT 0
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX federation_reader ON federation_objects (`published_at`);
CREATE TABLE federation_deliveries (
    `id` VARCHAR(191) PRIMARY KEY,
    `activity_id` VARCHAR(191) NOT NULL ,
    `inbox` VARCHAR(191) NOT NULL,
    `attempts` BIGINT NOT NULL DEFAULT 0,
    `next_attempt_at` VARCHAR(32) NOT NULL,
    `delivered_at` VARCHAR(32) ,
    `last_error` VARCHAR(1000) ,
    UNIQUE(activity_id,inbox)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
INSERT IGNORE INTO settings (id,`key`,value,type,`group`,created_at,updated_at)
VALUES ('native-social-enabled','social_web_enabled','false','boolean','social_web',UTC_TIMESTAMP(),UTC_TIMESTAMP());
ALTER TABLE federation_activities ADD COLUMN `undone` SMALLINT NOT NULL DEFAULT 0;
ALTER TABLE federation_received ADD COLUMN `read_at` VARCHAR(32);
CREATE TABLE federation_blocks (
    `actor` VARCHAR(191) PRIMARY KEY,
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
ALTER TABLE federation_received ADD COLUMN `undone` SMALLINT NOT NULL DEFAULT 0;

-- Foreign keys are added after the tables they point at.
-- SQLite resolves a reference lazily and accepts a forward one;
-- MySQL requires the referenced table to exist already.
ALTER TABLE federation_deliveries ADD CONSTRAINT federation_deliveries_activity_id_fk FOREIGN KEY (`activity_id`) REFERENCES federation_activities(`id`);
