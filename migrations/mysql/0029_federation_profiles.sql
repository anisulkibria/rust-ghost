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
