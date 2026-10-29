-- Member event history, signup attribution, and a location for a page view.
--
-- Of Ghost's eleven member event tables this schema had one,
-- `members_created_events`, and that one recorded only a bare `source`. So
-- the member detail page had no activity feed to show, the `/members/events`
-- feed had almost nothing to merge, and "which post brought this member in"
-- could not be answered at all.
--
-- The columns are Ghost's, so the same questions can be asked of the same
-- names. `attribution_*` is the resource a signup is credited to — a post, a
-- tag, the homepage — while `referrer_*` and `utm_*` are where the visitor
-- came from before that.

ALTER TABLE members_created_events ADD COLUMN attribution_id TEXT(24);
ALTER TABLE members_created_events ADD COLUMN attribution_type TEXT(50);
ALTER TABLE members_created_events ADD COLUMN attribution_url TEXT(2000);
ALTER TABLE members_created_events ADD COLUMN referrer_source TEXT(191);
ALTER TABLE members_created_events ADD COLUMN referrer_medium TEXT(191);
ALTER TABLE members_created_events ADD COLUMN referrer_url TEXT(2000);
ALTER TABLE members_created_events ADD COLUMN utm_source TEXT(191);
ALTER TABLE members_created_events ADD COLUMN utm_medium TEXT(191);
ALTER TABLE members_created_events ADD COLUMN utm_campaign TEXT(191);
ALTER TABLE members_created_events ADD COLUMN utm_term TEXT(191);
ALTER TABLE members_created_events ADD COLUMN utm_content TEXT(191);
CREATE INDEX IF NOT EXISTS members_created_events_attribution_idx
    ON members_created_events (attribution_type, attribution_id);

CREATE TABLE IF NOT EXISTS members_login_events (
    id TEXT PRIMARY KEY NOT NULL,
    member_id TEXT(24) NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    created_at DATETIME NOT NULL
);
CREATE INDEX IF NOT EXISTS members_login_events_member_idx ON members_login_events (member_id, created_at);

-- Free → paid and back. `from_status` is null for the first one, which is the
-- signup itself.
CREATE TABLE IF NOT EXISTS members_status_events (
    id TEXT PRIMARY KEY NOT NULL,
    member_id TEXT(24) NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    from_status TEXT(50),
    to_status TEXT(50) NOT NULL,
    created_at DATETIME NOT NULL
);
CREATE INDEX IF NOT EXISTS members_status_events_member_idx ON members_status_events (member_id, created_at);

CREATE TABLE IF NOT EXISTS members_email_change_events (
    id TEXT PRIMARY KEY NOT NULL,
    member_id TEXT(24) NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    from_email TEXT(191),
    to_email TEXT(191) NOT NULL,
    created_at DATETIME NOT NULL
);
CREATE INDEX IF NOT EXISTS members_email_change_events_member_idx ON members_email_change_events (member_id, created_at);

-- Subscribing and unsubscribing from one newsletter. `subscribed` is which
-- way it went, so an unsubscribe is an event rather than an absence.
CREATE TABLE IF NOT EXISTS members_subscribe_events (
    id TEXT PRIMARY KEY NOT NULL,
    member_id TEXT(24) NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    newsletter_id TEXT(24),
    subscribed BOOLEAN NOT NULL DEFAULT 1,
    source TEXT(50),
    created_at DATETIME NOT NULL
);
CREATE INDEX IF NOT EXISTS members_subscribe_events_member_idx ON members_subscribe_events (member_id, created_at);

-- A paid subscription starting, carrying the same attribution as a signup:
-- the question "which post sold this subscription" is the same shape as
-- "which post brought this member in".
CREATE TABLE IF NOT EXISTS members_subscription_created_events (
    id TEXT PRIMARY KEY NOT NULL,
    member_id TEXT(24) NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    subscription_id TEXT(24),
    attribution_id TEXT(24),
    attribution_type TEXT(50),
    attribution_url TEXT(2000),
    referrer_source TEXT(191),
    referrer_medium TEXT(191),
    referrer_url TEXT(2000),
    utm_source TEXT(191),
    utm_medium TEXT(191),
    utm_campaign TEXT(191),
    utm_term TEXT(191),
    utm_content TEXT(191),
    created_at DATETIME NOT NULL
);
CREATE INDEX IF NOT EXISTS members_subscription_created_events_member_idx
    ON members_subscription_created_events (member_id, created_at);
CREATE INDEX IF NOT EXISTS members_subscription_created_events_attribution_idx
    ON members_subscription_created_events (attribution_type, attribution_id);

-- A paid subscription changing: started, updated, canceled, expired.
-- `mrr_delta` is what the change did to monthly recurring revenue, which is
-- what makes an MRR series addable rather than recomputable.
CREATE TABLE IF NOT EXISTS members_paid_subscription_events (
    id TEXT PRIMARY KEY NOT NULL,
    member_id TEXT(24) NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    type TEXT(50) NOT NULL,
    subscription_id TEXT(24),
    from_plan TEXT(191),
    to_plan TEXT(191),
    from_status TEXT(50),
    to_status TEXT(50),
    currency TEXT(3),
    source TEXT(50),
    mrr_delta INTEGER NOT NULL DEFAULT 0,
    created_at DATETIME NOT NULL
);
CREATE INDEX IF NOT EXISTS members_paid_subscription_events_member_idx
    ON members_paid_subscription_events (member_id, created_at);
CREATE INDEX IF NOT EXISTS members_paid_subscription_events_created_idx
    ON members_paid_subscription_events (created_at);

CREATE TABLE IF NOT EXISTS members_payment_events (
    id TEXT PRIMARY KEY NOT NULL,
    member_id TEXT(24) NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    amount INTEGER NOT NULL DEFAULT 0,
    currency TEXT(3),
    source TEXT(50),
    created_at DATETIME NOT NULL
);
CREATE INDEX IF NOT EXISTS members_payment_events_member_idx ON members_payment_events (member_id, created_at);

-- A tier being added to or removed from a member.
CREATE TABLE IF NOT EXISTS members_product_events (
    id TEXT PRIMARY KEY NOT NULL,
    member_id TEXT(24) NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    product_id TEXT(24),
    action TEXT(50) NOT NULL,
    created_at DATETIME NOT NULL
);
CREATE INDEX IF NOT EXISTS members_product_events_member_idx ON members_product_events (member_id, created_at);

CREATE TABLE IF NOT EXISTS members_cancel_events (
    id TEXT PRIMARY KEY NOT NULL,
    member_id TEXT(24) NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    subscription_id TEXT(24),
    from_plan TEXT(191),
    created_at DATETIME NOT NULL
);
CREATE INDEX IF NOT EXISTS members_cancel_events_member_idx ON members_cancel_events (member_id, created_at);

-- Where a page view came from, beyond the referring host already stored:
-- the source and medium it resolves to, the campaign tags on the URL, and
-- the country, which the analytics page has no dimension for without it.
ALTER TABLE page_views ADD COLUMN referrer_source TEXT(191);
ALTER TABLE page_views ADD COLUMN referrer_medium TEXT(191);
ALTER TABLE page_views ADD COLUMN utm_source TEXT(191);
ALTER TABLE page_views ADD COLUMN utm_medium TEXT(191);
ALTER TABLE page_views ADD COLUMN utm_campaign TEXT(191);
ALTER TABLE page_views ADD COLUMN country TEXT(2);
CREATE INDEX IF NOT EXISTS page_views_country_idx ON page_views (country);
CREATE INDEX IF NOT EXISTS page_views_path_idx ON page_views (path, created_at);
