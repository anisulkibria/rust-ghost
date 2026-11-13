-- Members: offer redemptions, metafields, post feedback, and the member event
-- tables with their attribution.
--
-- Consolidated from earlier migrations. Each section below is one of them (or the
-- named part of one), unchanged and in the order it was first applied.

-- ============================================================================
-- offer_redemptions  (was 0012_offer_redemptions.sql)
-- ============================================================================

-- Which offer a member signed up through.
--
-- Ghost keeps redemptions in their own table so one member can redeem several
-- offers over time. A member here can only arrive through the one offer that
-- was on their first paid checkout, which is what the Offers screen counts, so
-- the link lives on the member row.
ALTER TABLE members ADD COLUMN offer_id TEXT(24) REFERENCES offers(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS members_offer_id_index ON members (offer_id);

-- ============================================================================
-- member_metafields  (was 0013_member_metafields.sql)
-- ============================================================================

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
    created_at TEXT NOT NULL,
    updated_at TEXT
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
    created_at TEXT NOT NULL,
    updated_at TEXT
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
    created_at TEXT NOT NULL,
    updated_at TEXT
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
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS members_metafield_change_events_member_id_index
    ON members_metafield_change_events (member_id);

-- ============================================================================
-- members_feedback  (was 0015_members_feedback.sql)
-- ============================================================================

-- Audience feedback. The "More like this / Less like this" buttons a
-- newsletter carries when its `feedback_enabled` switch is on link to
-- `#/feedback/{post}/{score}`, and Portal posts the click to
-- `/members/api/feedback`. Without somewhere to put it the link opened a modal
-- that could only fail, so the toggle in Settings → Membership promised
-- something the site could not keep.
--
-- Ghost's `members_feedback`, same columns. One row per member per post: a
-- reader changing their mind replaces their score rather than adding a second
-- one, which is what the unique index enforces.
CREATE TABLE IF NOT EXISTS members_feedback (
    id TEXT PRIMARY KEY NOT NULL,
    score INTEGER UNSIGNED NOT NULL DEFAULT 0,
    member_id TEXT(24) NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    post_id TEXT(24) NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    updated_at TEXT
);

CREATE UNIQUE INDEX IF NOT EXISTS members_feedback_member_post_idx
    ON members_feedback (member_id, post_id);

-- ============================================================================
-- member_events_and_attribution  (was 0026_member_events_and_attribution.sql)
-- ============================================================================

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
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS members_login_events_member_idx ON members_login_events (member_id, created_at);

-- Free → paid and back. `from_status` is null for the first one, which is the
-- signup itself.
CREATE TABLE IF NOT EXISTS members_status_events (
    id TEXT PRIMARY KEY NOT NULL,
    member_id TEXT(24) NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    from_status TEXT(50),
    to_status TEXT(50) NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS members_status_events_member_idx ON members_status_events (member_id, created_at);

CREATE TABLE IF NOT EXISTS members_email_change_events (
    id TEXT PRIMARY KEY NOT NULL,
    member_id TEXT(24) NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    from_email TEXT(191),
    to_email TEXT(191) NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS members_email_change_events_member_idx ON members_email_change_events (member_id, created_at);

-- Subscribing and unsubscribing from one newsletter. `subscribed` is which
-- way it went, so an unsubscribe is an event rather than an absence.
CREATE TABLE IF NOT EXISTS members_subscribe_events (
    id TEXT PRIMARY KEY NOT NULL,
    member_id TEXT(24) NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    newsletter_id TEXT(24),
    subscribed INTEGER NOT NULL DEFAULT 1,
    source TEXT(50),
    created_at TEXT NOT NULL
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
    created_at TEXT NOT NULL
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
    created_at TEXT NOT NULL
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
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS members_payment_events_member_idx ON members_payment_events (member_id, created_at);

-- A tier being added to or removed from a member.
CREATE TABLE IF NOT EXISTS members_product_events (
    id TEXT PRIMARY KEY NOT NULL,
    member_id TEXT(24) NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    product_id TEXT(24),
    action TEXT(50) NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS members_product_events_member_idx ON members_product_events (member_id, created_at);

CREATE TABLE IF NOT EXISTS members_cancel_events (
    id TEXT PRIMARY KEY NOT NULL,
    member_id TEXT(24) NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    subscription_id TEXT(24),
    from_plan TEXT(191),
    created_at TEXT NOT NULL
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
