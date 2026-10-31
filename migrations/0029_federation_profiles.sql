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
