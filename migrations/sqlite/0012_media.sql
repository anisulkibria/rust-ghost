-- The Media module.
--
-- The library itself is the filesystem: `content/images`, `content/media` and
-- `content/files` are the record of what exists, and the media manager reads
-- them directly. That is deliberate — an install that has been publishing for
-- years already has files there with no row to their name, and a table that
-- claimed to be the index would have been wrong about every one of them from
-- the moment it was created.
--
-- This table holds only what the filesystem cannot: the words put beside a
-- file. A file with nothing written about it has no row here at all, which is
-- the ordinary case.

CREATE TABLE media_meta (
 -- The stored path, family directory included: `images/2026/11/cover-ab12.jpg`.
 -- The key rather than a surrogate id, because the file is the thing being
 -- described and its path is what identifies it. Bounded by
 -- ghost_core::media::MAX_PATH, and only ever written in the form
 -- ghost_core::media::Location::path composes.
 path VARCHAR(400) PRIMARY KEY,
 -- Which of the three families the path begins with: 'images', 'media' or
 -- 'files'. Stored so the library can read every title for one family in a
 -- single query instead of filtering on a path prefix.
 kind VARCHAR(10) NOT NULL,
 -- A human name for the file, shown instead of the file name. Empty means
 -- "use the file name".
 title VARCHAR(191) NOT NULL DEFAULT '',
 -- Alternative text, for an image used in a post. Empty means none was
 -- written; it is not the same as a deliberately empty alt on a decorative
 -- image, and the editor's own card still owns that choice per use.
 alt VARCHAR(300) NOT NULL DEFAULT '',
 caption VARCHAR(500) NOT NULL DEFAULT '',
 created_at TEXT NOT NULL,
 updated_at TEXT NOT NULL
);

-- The library lists one family at a time, newest first.
CREATE INDEX media_meta_kind ON media_meta(kind);
