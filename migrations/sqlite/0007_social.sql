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
CREATE INDEX IF NOT EXISTS comments_parent_id_idx ON comments (parent_id, status);
CREATE INDEX IF NOT EXISTS comments_in_reply_to_id_idx ON comments (in_reply_to_id, status);
CREATE INDEX IF NOT EXISTS comments_member_id_idx ON comments (member_id, status);
CREATE INDEX IF NOT EXISTS comments_created_at_idx ON comments (created_at, id);

-- The unique indexes from 0001 cover `(comment_id, member_id)` lookups, but
-- not the `comment_id`-only counts that every listed comment needs.
CREATE INDEX IF NOT EXISTS comment_likes_comment_id_idx ON comment_likes (comment_id);
CREATE INDEX IF NOT EXISTS comment_reports_comment_id_idx ON comment_reports (comment_id);

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
    id TEXT PRIMARY KEY NOT NULL,
    source TEXT(2000) NOT NULL,
    source_title TEXT(2000),
    source_site_title TEXT(2000),
    source_excerpt TEXT(2000),
    source_author TEXT(2000),
    source_featured_image TEXT(2000),
    source_favicon TEXT(2000),
    target TEXT(2000) NOT NULL,
    resource_id TEXT(24),
    resource_type TEXT(50),
    payload TEXT,
    -- A mention the source page no longer makes. Kept rather than removed so
    -- a flapping source does not come back as a new mention each time.
    deleted INTEGER NOT NULL DEFAULT 0,
    -- Whether the source page was fetched and really does link here. An
    -- unverified mention is somebody claiming a link that is not there.
    verified INTEGER NOT NULL DEFAULT 0,
    revalidation_failure_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);
-- One mention per source/target pair: a source that sends the same webmention
-- twice is updating the one it already sent.
CREATE UNIQUE INDEX IF NOT EXISTS mentions_source_target_idx ON mentions (source, target);
CREATE INDEX IF NOT EXISTS mentions_resource_idx ON mentions (resource_type, resource_id);
CREATE INDEX IF NOT EXISTS mentions_created_idx ON mentions (created_at);

-- Webmentions this site has sent, so a republish does not re-notify every
-- link in the post and a failed send can be retried.
CREATE TABLE IF NOT EXISTS mention_sends (
    id TEXT PRIMARY KEY NOT NULL,
    source TEXT(2000) NOT NULL,
    target TEXT(2000) NOT NULL,
    endpoint TEXT(2000),
    status TEXT(50) NOT NULL DEFAULT 'pending',
    error TEXT(2000),
    created_at TEXT NOT NULL,
    updated_at TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS mention_sends_pair_idx ON mention_sends (source, target);

CREATE TABLE IF NOT EXISTS signing_keys (
    id TEXT PRIMARY KEY NOT NULL,
    private_pem TEXT NOT NULL,
    public_pem TEXT NOT NULL,
    created_at TEXT NOT NULL
);

-- +1 for a like, -1 for a dislike. Existing rows are likes, which is what
-- the default says.
ALTER TABLE comment_likes ADD COLUMN score INTEGER NOT NULL DEFAULT 1;

-- ============================================================================
-- native_federation  (was 0020_native_federation.sql)
-- ============================================================================

CREATE TABLE federation_keys (
    id TEXT PRIMARY KEY,
    private_pem TEXT NOT NULL,
    public_pem TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE federation_followers (
    actor TEXT PRIMARY KEY,
    inbox TEXT NOT NULL,
    follow_id TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE federation_following (
    actor TEXT PRIMARY KEY,
    inbox TEXT NOT NULL,
    follow_id TEXT NOT NULL,
    accepted INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);
CREATE TABLE federation_activities (
    id TEXT PRIMARY KEY,
    activity_type TEXT NOT NULL,
    object_id TEXT,
    body TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE federation_received (
    id TEXT PRIMARY KEY,
    actor TEXT NOT NULL,
    activity_type TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE federation_objects (
    id TEXT PRIMARY KEY,
    actor TEXT NOT NULL,
    object_type TEXT NOT NULL,
    content TEXT NOT NULL,
    url TEXT NOT NULL,
    in_reply_to TEXT,
    published_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    local INTEGER NOT NULL DEFAULT 0,
    deleted INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX federation_reader ON federation_objects(published_at);
CREATE TABLE federation_deliveries (
    id TEXT PRIMARY KEY,
    activity_id TEXT NOT NULL REFERENCES federation_activities(id),
    inbox TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TEXT NOT NULL,
    delivered_at TEXT,
    last_error TEXT,
    UNIQUE(activity_id,inbox)
);
INSERT OR IGNORE INTO settings (id,key,value,type,`group`,created_at,updated_at)
VALUES ('native-social-enabled','social_web_enabled','false','boolean','social_web',datetime('now'),datetime('now'));
ALTER TABLE federation_activities ADD COLUMN undone INTEGER NOT NULL DEFAULT 0;
ALTER TABLE federation_received ADD COLUMN read_at TEXT;
CREATE TABLE federation_blocks (
    actor TEXT PRIMARY KEY,
    created_at TEXT NOT NULL
);
ALTER TABLE federation_received ADD COLUMN undone INTEGER NOT NULL DEFAULT 0;

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
 actor TEXT PRIMARY KEY,
 handle TEXT NOT NULL,
 name TEXT,
 avatar TEXT,
 summary TEXT,
 url TEXT,
 followers INTEGER,
 following INTEGER,
 posts INTEGER,
 fetched_at TEXT NOT NULL
);

-- Likes and reposts arriving for a local note.
--
-- The inbox already authenticates and stores these as raw activities, but a
-- note's like count cannot be a JSON scan over every activity ever received.
-- One row per (actor, object, kind), so a double Like from a retrying server
-- counts once and an Undo can delete exactly one thing.
CREATE TABLE federation_interactions (
 id TEXT PRIMARY KEY,
 actor TEXT NOT NULL,
 object TEXT NOT NULL,
 kind TEXT NOT NULL,
 activity TEXT NOT NULL,
 created_at TEXT NOT NULL
);
CREATE UNIQUE INDEX federation_interactions_unique ON federation_interactions(actor, object, kind);
CREATE INDEX federation_interactions_object ON federation_interactions(object, kind);

-- Replies are looked up by what they answer, which is the one query a thread
-- view makes.
CREATE INDEX federation_objects_replies ON federation_objects(in_reply_to);
