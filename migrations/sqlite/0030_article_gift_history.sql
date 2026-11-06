-- The live association is separate from historical tokens, as in Ghost 6.73.
-- Legacy hashed links remain valid until their post's next reset/revocation.
CREATE TABLE gift_links (
    token TEXT NOT NULL PRIMARY KEY,
    post_id TEXT NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    updated_at TEXT
);
CREATE INDEX gift_links_post_id ON gift_links(post_id);
CREATE TABLE post_gift_links (
    post_id TEXT NOT NULL UNIQUE REFERENCES posts(id) ON DELETE CASCADE,
    gift_link_token TEXT NOT NULL PRIMARY KEY REFERENCES gift_links(token) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    updated_at TEXT
);
