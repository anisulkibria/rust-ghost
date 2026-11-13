-- Conversation and the social web: comments, mentions, signing keys and federation.
--
-- Consolidated from earlier migrations. Each section below is one of them (or the
-- named part of one), unchanged and in the order it was first applied.

-- ============================================================================
-- comments  (was 0009_comments.sql)
-- ============================================================================

-- Comments: the indexes the thread queries lean on.
--
-- The `comments` / `comment_likes` / `comment_reports` tables were created in
-- 0001 as schema-only placeholders, with just the `(post_id, status)` index
-- Ghost has. Reading a thread also walks the table by `parent_id` (replies),
-- by `in_reply_to_id` (the direct-reply count) and by `member_id` (the
-- moderation screen's per-member view), and each of those is a full scan
-- without an index — on a busy post that is the whole comments table per
-- rendered page.
CREATE INDEX comments_parent_id_idx ON comments (`parent_id`, `status`);
CREATE INDEX comments_in_reply_to_id_idx ON comments (`in_reply_to_id`, `status`);
CREATE INDEX comments_member_id_idx ON comments (`member_id`, `status`);
CREATE INDEX comments_created_at_idx ON comments (`created_at`, `id`);

-- The unique indexes from 0001 cover `(comment_id, member_id)` lookups, but
-- not the `comment_id`-only counts that every listed comment needs.
CREATE INDEX comment_likes_comment_id_idx ON comment_likes (`comment_id`);
CREATE INDEX comment_reports_comment_id_idx ON comment_reports (`comment_id`);

-- ============================================================================
-- mentions_identity_and_dislikes  (was 0027_mentions_identity_and_dislikes.sql)
-- ============================================================================

-- Webmentions, signing keys, and the other half of a comment reaction.
--
-- Three unrelated gaps that happen to need schema:
--
-- * `mentions` — Ghost's webmention store. Another site links to a post, tells
--   this one about it, and the mention is kept with enough of the source page
--   to show who it was. The same table is where incoming recommendations come
--   from: a recommendation is a mention whose source is another Ghost site's
--   `/.well-known/recommendations.json`.
-- * `signing_keys` — the keypair behind `/ghost/api/admin/identities` and
--   `/.well-known/jwks.json`, so another service can verify a token this site
--   issued. Kept apart from `federation_keys`, which is the publication's
--   ActivityPub identity and should not be reused for a different purpose.
-- * `comment_likes.score` — a reaction is +1 or -1. The table has held only
--   likes, so a dislike had nowhere to go.

CREATE TABLE IF NOT EXISTS mentions (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `source` VARCHAR(2000) NOT NULL,
    `source_title` VARCHAR(2000) ,
    `source_site_title` VARCHAR(2000) ,
    `source_excerpt` VARCHAR(2000) ,
    `source_author` VARCHAR(2000) ,
    `source_featured_image` VARCHAR(2000) ,
    `source_favicon` VARCHAR(2000) ,
    `target` VARCHAR(2000) NOT NULL,
    `resource_id` VARCHAR(24) ,
    `resource_type` VARCHAR(50) ,
    `payload` MEDIUMTEXT ,
    -- A mention the source page no longer makes. Kept rather than removed so
    -- a flapping source does not come back as a new mention each time.
    `deleted` SMALLINT NOT NULL DEFAULT 0,
    -- Whether the source page was fetched and really does link here. An
    -- unverified mention is somebody claiming a link that is not there.
    `verified` SMALLINT NOT NULL DEFAULT 0,
    `revalidation_failure_count` BIGINT NOT NULL DEFAULT 0,
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
-- One mention per source/target pair: a source that sends the same webmention
-- twice is updating the one it already sent.
CREATE UNIQUE INDEX mentions_source_target_idx ON mentions (`source`(191), `target`(191));
CREATE INDEX mentions_resource_idx ON mentions (`resource_type`, `resource_id`);
CREATE INDEX mentions_created_idx ON mentions (`created_at`);

-- Webmentions this site has sent, so a republish does not re-notify every
-- link in the post and a failed send can be retried.
CREATE TABLE IF NOT EXISTS mention_sends (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `source` VARCHAR(2000) NOT NULL,
    `target` VARCHAR(2000) NOT NULL,
    `endpoint` VARCHAR(2000) ,
    `status` VARCHAR(50) NOT NULL DEFAULT 'pending',
    `error` VARCHAR(2000) ,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE UNIQUE INDEX mention_sends_pair_idx ON mention_sends (`source`(191), `target`(191));

CREATE TABLE IF NOT EXISTS signing_keys (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `private_pem` VARCHAR(4000) NOT NULL,
    `public_pem` VARCHAR(4000) NOT NULL,
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- +1 for a like, -1 for a dislike. Existing rows are likes, which is what
-- the default says.
ALTER TABLE comment_likes ADD COLUMN `score` BIGINT NOT NULL DEFAULT 1;

-- ============================================================================
-- native_federation  (was 0020_native_federation.sql)
-- ============================================================================

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

-- ============================================================================
-- federation_profiles  (was 0029_federation_profiles.sql)
-- ============================================================================

-- Remote actor profiles, cached.
--
-- Everything the social web screens show about somebody — their handle, their
-- display name, their avatar — lives on their own server, behind a fetch. The
-- admin cannot make that fetch while rendering a list of fifty accounts, so
-- what it has been showing instead is the raw actor URL. This is where a
-- profile is kept once it has been fetched, so a handle can be rendered from
-- the database at list speed.
--
-- `fetched_at` is what makes a row refreshable rather than permanent: a
-- display name and an avatar change, and a cache with no age cannot tell a
-- current profile from a three-year-old one.
CREATE TABLE federation_actors (
 `actor` VARCHAR(191) PRIMARY KEY,
 `handle` VARCHAR(1000) NOT NULL,
 `name` VARCHAR(1000) ,
 `avatar` VARCHAR(1000) ,
 `summary` VARCHAR(4000) ,
 `url` VARCHAR(1000) ,
 `followers` BIGINT ,
 `following` BIGINT ,
 `posts` BIGINT ,
 `fetched_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Likes and reposts arriving for a local note.
--
-- The inbox already authenticates and stores these as raw activities, but a
-- note's like count cannot be a JSON scan over every activity ever received.
-- One row per (actor, object, kind), so a double Like from a retrying server
-- counts once and an Undo can delete exactly one thing.
CREATE TABLE federation_interactions (
 `id` VARCHAR(191) PRIMARY KEY,
 `actor` VARCHAR(191) NOT NULL,
 `object` VARCHAR(191) NOT NULL,
 `kind` VARCHAR(191) NOT NULL,
 `activity` VARCHAR(4000) NOT NULL,
 `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE UNIQUE INDEX federation_interactions_unique ON federation_interactions (`actor`, `object`, `kind`);
CREATE INDEX federation_interactions_object ON federation_interactions (`object`, `kind`);

-- Replies are looked up by what they answer, which is the one query a thread
-- view makes.
CREATE INDEX federation_objects_replies ON federation_objects (`in_reply_to`);
