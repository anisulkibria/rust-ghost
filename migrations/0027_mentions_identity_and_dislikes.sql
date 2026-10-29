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
    deleted BOOLEAN NOT NULL DEFAULT 0,
    -- Whether the source page was fetched and really does link here. An
    -- unverified mention is somebody claiming a link that is not there.
    verified BOOLEAN NOT NULL DEFAULT 0,
    revalidation_failure_count INTEGER NOT NULL DEFAULT 0,
    created_at DATETIME NOT NULL
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
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);
CREATE UNIQUE INDEX IF NOT EXISTS mention_sends_pair_idx ON mention_sends (source, target);

CREATE TABLE IF NOT EXISTS signing_keys (
    id TEXT PRIMARY KEY NOT NULL,
    private_pem TEXT NOT NULL,
    public_pem TEXT NOT NULL,
    created_at DATETIME NOT NULL
);

-- +1 for a like, -1 for a dislike. Existing rows are likes, which is what
-- the default says.
ALTER TABLE comment_likes ADD COLUMN score INTEGER NOT NULL DEFAULT 1;
