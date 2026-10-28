-- Persistent workflows shared by the admin UI, public endpoints and APIs.
ALTER TABLE post_revisions ADD COLUMN snapshot TEXT;
CREATE TABLE email_suppressions (
    email TEXT PRIMARY KEY COLLATE NOCASE,
    reason TEXT NOT NULL,
    provider TEXT NOT NULL DEFAULT 'local',
    created_at TEXT NOT NULL
);
CREATE TABLE email_delivery_events (
    id TEXT PRIMARY KEY,
    event_type TEXT NOT NULL,
    email TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE newsletter_links (
    id TEXT PRIMARY KEY,
    post_id TEXT NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
    from_url TEXT NOT NULL,
    to_url TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(post_id, from_url)
);
CREATE TABLE newsletter_clicks (
    id TEXT PRIMARY KEY,
    link_id TEXT NOT NULL REFERENCES newsletter_links(id) ON DELETE CASCADE,
    member_uuid TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX newsletter_clicks_link ON newsletter_clicks(link_id, created_at);
CREATE TABLE saved_views (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    resource TEXT NOT NULL,
    name TEXT NOT NULL,
    filter TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(user_id, resource, name)
);
CREATE TABLE article_gift_links (
    id TEXT PRIMARY KEY,
    post_id TEXT NOT NULL UNIQUE REFERENCES posts(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);
CREATE TABLE staff_password_resets (
    token_hash TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE scheduled_gift_deliveries (
    token TEXT PRIMARY KEY,
    deliver_at TEXT NOT NULL,
    sent_at TEXT,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT
);
ALTER TABLE labels ADD COLUMN slug TEXT;
UPDATE labels SET slug=lower(replace(name,' ','-')) WHERE slug IS NULL;
CREATE TRIGGER labels_slug_on_insert AFTER INSERT ON labels WHEN NEW.slug IS NULL
BEGIN UPDATE labels SET slug=lower(replace(NEW.name,' ','-')) WHERE id=NEW.id; END;
ALTER TABLE page_views ADD COLUMN device TEXT NOT NULL DEFAULT 'unknown';
ALTER TABLE page_views ADD COLUMN browser TEXT NOT NULL DEFAULT 'unknown';
CREATE INDEX page_views_realtime ON page_views(created_at,viewer);
ALTER TABLE scheduled_gift_deliveries ADD COLUMN last_attempt_at TEXT;
ALTER TABLE posts ADD COLUMN newsletter_audience_filter TEXT;
ALTER TABLE posts ADD COLUMN newsletter_include_preview BOOLEAN NOT NULL DEFAULT 0;
ALTER TABLE members_products ADD COLUMN subscription_id TEXT;
CREATE INDEX members_products_subscription ON members_products(subscription_id);
-- Existing recurring grants can be attributed where their price mapping is known.
UPDATE members_products SET subscription_id=(
    SELECT s.subscription_id FROM subscriptions s
    JOIN members_stripe_customers c ON c.customer_id=s.customer_id
    JOIN products p ON p.id=members_products.product_id
    WHERE c.member_id=members_products.member_id AND s.status IN ('active','trialing')
      AND (p.monthly_price_id=s.stripe_price_id OR p.yearly_price_id=s.stripe_price_id)
    ORDER BY s.created_at DESC LIMIT 1
) WHERE expiry_at IS NULL;
INSERT OR IGNORE INTO settings (id,`group`,key,value,type,created_at,updated_at)
VALUES (lower(hex(randomblob(12))),'email','mailgun_webhook_signing_key','','string',datetime('now'),datetime('now'));
