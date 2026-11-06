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
