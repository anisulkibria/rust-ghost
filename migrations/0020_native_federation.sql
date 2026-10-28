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
    accepted BOOLEAN NOT NULL DEFAULT 0,
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
    local BOOLEAN NOT NULL DEFAULT 0,
    deleted BOOLEAN NOT NULL DEFAULT 0
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
ALTER TABLE federation_activities ADD COLUMN undone BOOLEAN NOT NULL DEFAULT 0;
ALTER TABLE federation_received ADD COLUMN read_at TEXT;
CREATE TABLE federation_blocks (
    actor TEXT PRIMARY KEY,
    created_at TEXT NOT NULL
);
ALTER TABLE federation_received ADD COLUMN undone BOOLEAN NOT NULL DEFAULT 0;
