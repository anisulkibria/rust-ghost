-- Member metafields: the custom fields a publisher defines on their members.
--
-- Ghost 6.72's schema, kept whole (core/server/data/schema/schema.js). The
-- feature was called "custom fields" until 6.63 renamed the tables; these are
-- the post-rename names.

-- The definitions. One row per field a site has declared.
CREATE TABLE IF NOT EXISTS members_metafields (
    id TEXT(24) PRIMARY KEY NOT NULL,
    -- The stable handle. Values reference this rather than the id, so a field
    -- can be renamed without rewriting every value.
    key TEXT(191) NOT NULL UNIQUE,
    name TEXT(191) NOT NULL UNIQUE,
    -- short_text | long_text | address
    type TEXT(50) NOT NULL,
    -- active | archived
    status TEXT(50) NOT NULL DEFAULT 'active',
    -- none | read | write — what the member themselves may do with it
    member_access TEXT(50) NOT NULL DEFAULT 'none',
    -- The publisher's order. Only the relative order carries meaning: creates
    -- append past the highest rank and deletes leave gaps, so reads tie-break
    -- on created_at.
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);

-- One row per leaf: a scalar field stores its value under the empty path, a
-- record stores one row per part the member filled in.
CREATE TABLE IF NOT EXISTS members_metafield_values (
    id TEXT(24) PRIMARY KEY NOT NULL,
    -- The field's key, not its id: a value is addressed by key everywhere it
    -- matters, so the row carries it directly and reads skip a join.
    metafield_key TEXT(191) NOT NULL REFERENCES members_metafields(key) ON DELETE CASCADE,
    member_id TEXT(24) NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    -- Which part of the value this row carries; '' for a scalar.
    path TEXT(191) NOT NULL DEFAULT '',
    -- A part a member has not filled in has no row at all.
    value_text TEXT,
    -- Who wrote what is here now. Shaped like `actions`: a type and an id, and
    -- no foreign key, because provenance has to outlive its cause.
    written_by_type TEXT(50) NOT NULL,
    written_by_id TEXT(24),
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);

CREATE UNIQUE INDEX IF NOT EXISTS members_metafield_values_leaf_unique
    ON members_metafield_values (member_id, metafield_key, path);
-- What a segment filter looks up: every member holding a value for a given
-- part of a given field.
CREATE INDEX IF NOT EXISTS members_metafield_values_key_path_index
    ON members_metafield_values (metafield_key, path);

-- Where an external source's data lands. Several ports may route into one
-- field, so the key is indexed rather than unique.
CREATE TABLE IF NOT EXISTS members_metafield_bindings (
    id TEXT(24) PRIMARY KEY NOT NULL,
    port TEXT(191) NOT NULL UNIQUE,
    metafield_key TEXT(191) NOT NULL REFERENCES members_metafields(key) ON DELETE CASCADE,
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);

CREATE INDEX IF NOT EXISTS members_metafield_bindings_key_index
    ON members_metafield_bindings (metafield_key);

-- A member's activity feed entry for a change to their fields.
CREATE TABLE IF NOT EXISTS members_metafield_change_events (
    id TEXT(24) PRIMARY KEY NOT NULL,
    member_id TEXT(24) NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    written_by_type TEXT(50) NOT NULL,
    written_by_id TEXT(24),
    -- Where the change was made (admin, portal, import…). Separate from who
    -- made it: a member can supply values somewhere other than their account.
    source TEXT(50) NOT NULL,
    -- The fields changed, as JSON carrying each field's namespace, key and the
    -- name it had at the time — copied rather than joined, so an entry still
    -- reads after a field is renamed or deleted.
    metafields TEXT NOT NULL,
    created_at DATETIME NOT NULL
);

CREATE INDEX IF NOT EXISTS members_metafield_change_events_member_id_index
    ON members_metafield_change_events (member_id);
