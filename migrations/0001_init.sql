-- Ghost-rust initial schema.
-- Column names/types mirror Ghost's server/data/schema/schema.js so that a
-- real Ghost SQLite database can be imported via INSERT INTO ... SELECT.
-- PKs are app-generated 24-hex ObjectIds; booleans are 0/1; dates are
-- TEXT 'YYYY-MM-DD HH:mm:ss' UTC.

CREATE TABLE IF NOT EXISTS posts (
    id TEXT PRIMARY KEY NOT NULL,
    uuid TEXT(36),
    title TEXT,
    slug TEXT NOT NULL,
    mobiledoc TEXT,
    lexical TEXT,
    html TEXT,
    comment_id TEXT(50),
    plaintext TEXT,
    feature_image TEXT(2000),
    featured BOOLEAN NOT NULL DEFAULT 0,
    type TEXT(50) NOT NULL DEFAULT 'post',
    status TEXT(50) NOT NULL DEFAULT 'draft',
    locale TEXT(6),
    visibility TEXT(50) NOT NULL DEFAULT 'public',
    email_recipient_filter TEXT,
    created_at DATETIME NOT NULL,
    updated_at DATETIME,
    published_at DATETIME,
    published_by TEXT(24),
    custom_excerpt TEXT(2000),
    auto_excerpt TEXT(500),
    reading_time INTEGER UNSIGNED,
    codeinjection_head TEXT,
    codeinjection_foot TEXT,
    custom_template TEXT(100),
    canonical_url TEXT(2000),
    newsletter_id TEXT(24) REFERENCES newsletters(id),
    show_title_and_feature_image BOOLEAN NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS posts_uuid_idx ON posts (uuid);
CREATE INDEX IF NOT EXISTS posts_published_at_idx ON posts (published_at);
CREATE INDEX IF NOT EXISTS posts_updated_at_idx ON posts (updated_at);
CREATE INDEX IF NOT EXISTS posts_type_status_idx ON posts (type, status, updated_at);
CREATE UNIQUE INDEX IF NOT EXISTS posts_slug_type_unique ON posts (slug, type);

CREATE TABLE IF NOT EXISTS posts_meta (
    id TEXT PRIMARY KEY NOT NULL,
    post_id TEXT(24) NOT NULL UNIQUE REFERENCES posts(id) ON DELETE CASCADE,
    og_image TEXT(2000),
    og_title TEXT(300),
    og_description TEXT(500),
    twitter_image TEXT(2000),
    twitter_title TEXT(300),
    twitter_description TEXT(500),
    meta_title TEXT(300),
    meta_description TEXT(500),
    email_subject TEXT(300),
    email_only BOOLEAN NOT NULL DEFAULT 0,
    email_preview TEXT(300),
    frontmatter TEXT,
    feature_image_alt TEXT(2000),
    feature_image_caption TEXT
);

CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT(191) NOT NULL,
    slug TEXT(191) NOT NULL UNIQUE,
    password TEXT(60) NOT NULL,
    email TEXT(191) NOT NULL UNIQUE,
    profile_image TEXT(2000),
    cover_image TEXT(2000),
    bio TEXT(2000),
    website TEXT(2000),
    location TEXT(150),
    facebook TEXT(2000),
    twitter TEXT(2000),
    threads TEXT(191),
    bluesky TEXT(191),
    mastodon TEXT(191),
    tiktok TEXT(191),
    youtube TEXT(191),
    instagram TEXT(191),
    linkedin TEXT(191),
    accessibility TEXT,
    status TEXT(50) NOT NULL DEFAULT 'active',
    locale TEXT(6),
    visibility TEXT(50) NOT NULL DEFAULT 'public',
    meta_title TEXT(300),
    meta_description TEXT(500),
    tour TEXT,
    last_seen DATETIME,
    comment_notifications BOOLEAN NOT NULL DEFAULT 1,
    free_member_signup_notification BOOLEAN NOT NULL DEFAULT 1,
    paid_subscription_started_notification BOOLEAN NOT NULL DEFAULT 1,
    paid_subscription_canceled_notification BOOLEAN NOT NULL DEFAULT 0,
    mention_notifications BOOLEAN NOT NULL DEFAULT 1,
    recommendation_notifications BOOLEAN NOT NULL DEFAULT 1,
    milestone_notifications BOOLEAN NOT NULL DEFAULT 1,
    donation_notifications BOOLEAN NOT NULL DEFAULT 1,
    gift_subscription_notifications BOOLEAN NOT NULL DEFAULT 1,
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);

CREATE TABLE IF NOT EXISTS tags (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT(191) NOT NULL,
    slug TEXT(191) NOT NULL UNIQUE,
    description TEXT(500),
    feature_image TEXT(2000),
    parent_id TEXT(24),
    visibility TEXT(50) NOT NULL DEFAULT 'public',
    og_image TEXT(2000),
    og_title TEXT(300),
    og_description TEXT(500),
    twitter_image TEXT(2000),
    twitter_title TEXT(300),
    twitter_description TEXT(500),
    meta_title TEXT(300),
    meta_description TEXT(500),
    codeinjection_head TEXT,
    codeinjection_foot TEXT,
    canonical_url TEXT(2000),
    accent_color TEXT(50),
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);

CREATE TABLE IF NOT EXISTS posts_tags (
    id TEXT PRIMARY KEY NOT NULL,
    post_id TEXT(24) NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
    tag_id TEXT(24) NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    sort_order INTEGER UNSIGNED NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS posts_tags_post_id_tag_id_idx ON posts_tags (post_id, tag_id);

CREATE TABLE IF NOT EXISTS posts_authors (
    id TEXT PRIMARY KEY NOT NULL,
    post_id TEXT(24) NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
    author_id TEXT(24) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    sort_order INTEGER UNSIGNED NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS posts_authors_post_id_author_id_idx ON posts_authors (post_id, author_id);

CREATE TABLE IF NOT EXISTS roles (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT(50) NOT NULL UNIQUE,
    description TEXT(2000),
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);

CREATE TABLE IF NOT EXISTS roles_users (
    id TEXT PRIMARY KEY NOT NULL,
    role_id TEXT(24) NOT NULL,
    user_id TEXT(24) NOT NULL
);

CREATE TABLE IF NOT EXISTS permissions (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT(50) NOT NULL UNIQUE,
    object_type TEXT(50) NOT NULL,
    action_type TEXT(50) NOT NULL,
    object_id TEXT(24),
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);

CREATE TABLE IF NOT EXISTS permissions_users (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT(24) NOT NULL,
    permission_id TEXT(24) NOT NULL
);

CREATE TABLE IF NOT EXISTS permissions_roles (
    id TEXT PRIMARY KEY NOT NULL,
    role_id TEXT(24) NOT NULL,
    permission_id TEXT(24) NOT NULL
);

CREATE TABLE IF NOT EXISTS settings (
    id TEXT PRIMARY KEY NOT NULL,
    `group` TEXT(50) NOT NULL DEFAULT 'core',
    key TEXT(50) NOT NULL UNIQUE,
    value TEXT,
    type TEXT(50) NOT NULL DEFAULT 'string',
    flags TEXT(50),
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT(32) NOT NULL UNIQUE,
    user_id TEXT(24) NOT NULL,
    session_data TEXT(2000) NOT NULL,
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);

CREATE TABLE IF NOT EXISTS integrations (
    id TEXT PRIMARY KEY NOT NULL,
    type TEXT(50) NOT NULL DEFAULT 'custom',
    name TEXT(191) NOT NULL,
    slug TEXT(191) NOT NULL UNIQUE,
    icon_image TEXT(2000),
    description TEXT(2000),
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);

CREATE TABLE IF NOT EXISTS webhooks (
    id TEXT PRIMARY KEY NOT NULL,
    event TEXT(50) NOT NULL,
    target_url TEXT(2000) NOT NULL,
    name TEXT(191),
    secret TEXT(191),
    api_version TEXT(50) NOT NULL DEFAULT 'v2',
    integration_id TEXT(24) NOT NULL REFERENCES integrations(id) ON DELETE CASCADE,
    last_triggered_at DATETIME,
    last_triggered_status TEXT(50),
    last_triggered_error TEXT(50),
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);

CREATE TABLE IF NOT EXISTS api_keys (
    id TEXT PRIMARY KEY NOT NULL,
    type TEXT(50) NOT NULL,
    secret TEXT(191) NOT NULL UNIQUE,
    role_id TEXT(24),
    integration_id TEXT(24) REFERENCES integrations(id) ON DELETE CASCADE,
    user_id TEXT(24),
    last_seen_at DATETIME,
    last_seen_version TEXT(50),
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);

CREATE TABLE IF NOT EXISTS actions (
    id TEXT PRIMARY KEY NOT NULL,
    resource_id TEXT(24),
    resource_type TEXT(50) NOT NULL,
    actor_id TEXT(24) NOT NULL,
    actor_type TEXT(50) NOT NULL,
    event TEXT(50) NOT NULL,
    context TEXT,
    created_at DATETIME NOT NULL
);
CREATE INDEX IF NOT EXISTS actions_resource_idx ON actions (resource_type, resource_id);

CREATE TABLE IF NOT EXISTS invites (
    id TEXT PRIMARY KEY NOT NULL,
    role_id TEXT(24) NOT NULL,
    token TEXT(191) NOT NULL UNIQUE,
    expires BIGINT NOT NULL,
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);

CREATE TABLE IF NOT EXISTS brute (
    key TEXT(191) PRIMARY KEY NOT NULL,
    firstRequest BIGINT,
    lastRequest BIGINT,
    lifetime BIGINT,
    count INTEGER
);

CREATE TABLE IF NOT EXISTS tokens (
    id TEXT PRIMARY KEY NOT NULL,
    token TEXT(32) NOT NULL,
    uuid TEXT(36) UNIQUE,
    data TEXT(2000),
    created_at DATETIME NOT NULL,
    updated_at DATETIME,
    first_used_at DATETIME,
    used_count INTEGER UNSIGNED NOT NULL DEFAULT 0,
    otc_used_count INTEGER UNSIGNED NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS tokens_token_idx ON tokens (token);

CREATE TABLE IF NOT EXISTS snippets (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT(191) NOT NULL UNIQUE,
    mobiledoc TEXT NOT NULL,
    lexical TEXT,
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);

CREATE TABLE IF NOT EXISTS custom_theme_settings (
    id TEXT PRIMARY KEY NOT NULL,
    theme TEXT(191) NOT NULL,
    key TEXT(191) NOT NULL,
    value TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS custom_theme_settings_theme_key_unique ON custom_theme_settings (theme, key);

-- ---------------------------------------------------------------------------
-- Deferred-phase tables (members/newsletters/emails/comments/offers/tiers).
-- Schema-only so future phases and Ghost DB imports have somewhere to land.
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS members (
    id TEXT PRIMARY KEY NOT NULL,
    uuid TEXT(36) UNIQUE,
    transient_id TEXT(191) NOT NULL UNIQUE,
    email TEXT(191) NOT NULL UNIQUE,
    status TEXT(50) NOT NULL DEFAULT 'free',
    name TEXT(191),
    expertise TEXT(50),
    note TEXT(2000),
    geolocation TEXT(2000),
    enable_comment_notifications BOOLEAN NOT NULL DEFAULT 1,
    enable_updates_and_announcements BOOLEAN,
    email_count INTEGER UNSIGNED NOT NULL DEFAULT 0,
    email_opened_count INTEGER UNSIGNED NOT NULL DEFAULT 0,
    email_open_rate INTEGER UNSIGNED,
    email_disabled BOOLEAN NOT NULL DEFAULT 0,
    last_seen_at DATETIME,
    last_commented_at DATETIME,
    commenting TEXT,
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);
CREATE INDEX IF NOT EXISTS members_created_at_idx ON members (created_at, id);
CREATE INDEX IF NOT EXISTS members_email_disabled_idx ON members (email_disabled);

CREATE TABLE IF NOT EXISTS products (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT(191) NOT NULL,
    slug TEXT(191) NOT NULL UNIQUE,
    active BOOLEAN NOT NULL DEFAULT 1,
    welcome_page_url TEXT(2000),
    visibility TEXT(50) NOT NULL DEFAULT 'none',
    trial_days INTEGER UNSIGNED NOT NULL DEFAULT 0,
    description TEXT(191),
    type TEXT(50) NOT NULL DEFAULT 'paid',
    currency TEXT(50),
    monthly_price INTEGER UNSIGNED,
    yearly_price INTEGER UNSIGNED,
    monthly_price_id TEXT(24),
    yearly_price_id TEXT(24),
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);

CREATE TABLE IF NOT EXISTS members_products (
    id TEXT PRIMARY KEY NOT NULL,
    member_id TEXT(24) NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    product_id TEXT(24) NOT NULL REFERENCES products(id) ON DELETE CASCADE,
    sort_order INTEGER UNSIGNED NOT NULL DEFAULT 0,
    expiry_at DATETIME
);

CREATE TABLE IF NOT EXISTS posts_products (
    id TEXT PRIMARY KEY NOT NULL,
    post_id TEXT(24) NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
    product_id TEXT(24) NOT NULL REFERENCES products(id) ON DELETE CASCADE,
    sort_order INTEGER UNSIGNED NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS benefits (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT(191) NOT NULL,
    slug TEXT(191) NOT NULL UNIQUE,
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);

CREATE TABLE IF NOT EXISTS products_benefits (
    id TEXT PRIMARY KEY NOT NULL,
    product_id TEXT(24) NOT NULL REFERENCES products(id) ON DELETE CASCADE,
    benefit_id TEXT(24) NOT NULL REFERENCES benefits(id) ON DELETE CASCADE,
    sort_order INTEGER UNSIGNED NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS offers (
    id TEXT PRIMARY KEY NOT NULL,
    active BOOLEAN NOT NULL DEFAULT 0,
    name TEXT(191) NOT NULL UNIQUE,
    code TEXT(191) NOT NULL UNIQUE,
    product_id TEXT(24) NOT NULL REFERENCES products(id) ON DELETE CASCADE,
    stripe_coupon_id TEXT(255) UNIQUE,
    interval TEXT(50),
    currency TEXT(50),
    discount_type TEXT(50),
    discount_amount INTEGER,
    duration TEXT(50),
    duration_in_months INTEGER,
    portal_title TEXT(191),
    portal_description TEXT(2000),
    redemption_type TEXT(50),
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);

CREATE TABLE IF NOT EXISTS newsletters (
    id TEXT PRIMARY KEY NOT NULL,
    uuid TEXT(36),
    name TEXT(191) NOT NULL UNIQUE,
    description TEXT(2000),
    feedback_enabled BOOLEAN NOT NULL DEFAULT 0,
    slug TEXT(191) NOT NULL UNIQUE,
    sender_name TEXT(191),
    sender_email TEXT(191),
    sender_reply_to TEXT(191) NOT NULL DEFAULT 'newsletter',
    status TEXT(50) NOT NULL DEFAULT 'active',
    visibility TEXT(50) NOT NULL DEFAULT 'members',
    subscribe_on_signup BOOLEAN NOT NULL DEFAULT 1,
    sort_order INTEGER UNSIGNED NOT NULL DEFAULT 0,
    header_image TEXT(2000),
    show_header_icon BOOLEAN NOT NULL DEFAULT 1,
    show_header_title BOOLEAN NOT NULL DEFAULT 1,
    show_excerpt BOOLEAN NOT NULL DEFAULT 0,
    show_feature_image BOOLEAN NOT NULL DEFAULT 1,
    footer_content TEXT,
    show_badge BOOLEAN NOT NULL DEFAULT 1,
    show_header_name BOOLEAN NOT NULL DEFAULT 1,
    show_post_title_section BOOLEAN NOT NULL DEFAULT 1,
    show_comment_cta BOOLEAN NOT NULL DEFAULT 1,
    show_subscription_details BOOLEAN NOT NULL DEFAULT 0,
    show_latest_posts BOOLEAN NOT NULL DEFAULT 0,
    show_share_button BOOLEAN NOT NULL DEFAULT 0,
    background_color TEXT(50) NOT NULL DEFAULT 'light',
    post_title_color TEXT(50),
    header_background_color TEXT(50) NOT NULL DEFAULT 'transparent',
    section_title_color TEXT(50),
    divider_color TEXT(50),
    button_color TEXT(50) DEFAULT 'accent',
    link_color TEXT(50) DEFAULT 'accent',
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);

CREATE TABLE IF NOT EXISTS members_newsletters (
    id TEXT PRIMARY KEY NOT NULL,
    member_id TEXT(24) NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    newsletter_id TEXT(24) NOT NULL REFERENCES newsletters(id) ON DELETE CASCADE,
    status TEXT(50) NOT NULL DEFAULT 'confirmed'
);

CREATE TABLE IF NOT EXISTS emails (
    id TEXT PRIMARY KEY NOT NULL,
    post_id TEXT(24) NOT NULL UNIQUE,
    status TEXT(50) NOT NULL DEFAULT 'pending',
    recipient_filter TEXT,
    error TEXT(2000),
    error_data TEXT,
    email_count INTEGER UNSIGNED NOT NULL DEFAULT 0,
    delivered_count INTEGER UNSIGNED NOT NULL DEFAULT 0,
    opened_count INTEGER UNSIGNED NOT NULL DEFAULT 0,
    failed_count INTEGER UNSIGNED NOT NULL DEFAULT 0,
    subject TEXT(300),
    `from` TEXT(2000),
    reply_to TEXT(2000),
    html TEXT,
    plaintext TEXT,
    source TEXT,
    track_opens BOOLEAN NOT NULL DEFAULT 0,
    track_clicks BOOLEAN NOT NULL DEFAULT 0,
    feedback_enabled BOOLEAN NOT NULL DEFAULT 0,
    submitted_at DATETIME,
    newsletter_id TEXT(24) REFERENCES newsletters(id),
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);

CREATE TABLE IF NOT EXISTS email_batches (
    id TEXT PRIMARY KEY NOT NULL,
    email_id TEXT(24) NOT NULL REFERENCES emails(id) ON DELETE CASCADE,
    provider_id TEXT(191),
    status TEXT(50) NOT NULL DEFAULT 'submitted',
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);

CREATE TABLE IF NOT EXISTS email_recipients (
    id TEXT PRIMARY KEY NOT NULL,
    email_id TEXT(24) NOT NULL REFERENCES emails(id) ON DELETE CASCADE,
    member_id TEXT(24),
    batch_id TEXT(24),
    status TEXT(50) NOT NULL DEFAULT 'pending',
    delivered_at DATETIME,
    opened_at DATETIME,
    failed_at DATETIME,
    error_code INTEGER,
    error TEXT(2000),
    email_address TEXT(191),
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);
CREATE INDEX IF NOT EXISTS email_recipients_email_member_idx ON email_recipients (email_id, member_id);

CREATE TABLE IF NOT EXISTS email_recipient_failures (
    id TEXT PRIMARY KEY NOT NULL,
    email_id TEXT(24) NOT NULL REFERENCES emails(id) ON DELETE CASCADE,
    email_recipient_id TEXT(24),
    member_id TEXT(24),
    message TEXT(2000),
    failed_at DATETIME
);

CREATE TABLE IF NOT EXISTS comments (
    id TEXT PRIMARY KEY NOT NULL,
    post_id TEXT(24) NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
    member_id TEXT(24) REFERENCES members(id) ON DELETE SET NULL,
    parent_id TEXT(24) REFERENCES comments(id) ON DELETE CASCADE,
    in_reply_to_id TEXT(24) REFERENCES comments(id) ON DELETE SET NULL,
    status TEXT(50) NOT NULL DEFAULT 'published',
    html TEXT,
    edited_at DATETIME,
    pinned_at DATETIME,
    created_at DATETIME NOT NULL,
    updated_at DATETIME NOT NULL
);
CREATE INDEX IF NOT EXISTS comments_post_id_idx ON comments (post_id, status);

CREATE TABLE IF NOT EXISTS comment_likes (
    id TEXT PRIMARY KEY NOT NULL,
    comment_id TEXT(24) NOT NULL REFERENCES comments(id) ON DELETE CASCADE,
    member_id TEXT(24) NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    created_at DATETIME NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS comment_likes_unique ON comment_likes (comment_id, member_id);

CREATE TABLE IF NOT EXISTS comment_reports (
    id TEXT PRIMARY KEY NOT NULL,
    comment_id TEXT(24) NOT NULL REFERENCES comments(id) ON DELETE CASCADE,
    member_id TEXT(24) NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    created_at DATETIME NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS comment_reports_unique ON comment_reports (comment_id, member_id);

CREATE TABLE IF NOT EXISTS labels (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT(191) NOT NULL,
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);
CREATE UNIQUE INDEX IF NOT EXISTS labels_name_unique ON labels (name);

CREATE TABLE IF NOT EXISTS members_labels (
    id TEXT PRIMARY KEY NOT NULL,
    label_id TEXT(24) NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
    member_id TEXT(24) NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    sort_order INTEGER UNSIGNED NOT NULL DEFAULT 0
);
CREATE UNIQUE INDEX IF NOT EXISTS members_labels_unique ON members_labels (member_id, label_id);

CREATE TABLE IF NOT EXISTS mobiledoc_revisions (
    id TEXT PRIMARY KEY NOT NULL,
    post_id TEXT(24) NOT NULL,
    mobiledoc TEXT,
    created_at_ts BIGINT NOT NULL,
    created_at DATETIME NOT NULL
);
CREATE INDEX IF NOT EXISTS mobiledoc_revisions_post_idx ON mobiledoc_revisions (post_id);

CREATE TABLE IF NOT EXISTS post_revisions (
    id TEXT PRIMARY KEY NOT NULL,
    post_id TEXT(24) NOT NULL,
    lexical TEXT,
    created_at_ts BIGINT NOT NULL,
    created_at DATETIME NOT NULL,
    author_id TEXT(24) REFERENCES users(id),
    reason TEXT(50),
    feature_image TEXT(2000),
    feature_image_caption TEXT
);
CREATE INDEX IF NOT EXISTS post_revisions_post_idx ON post_revisions (post_id);

CREATE TABLE IF NOT EXISTS milestones (
    id TEXT PRIMARY KEY NOT NULL,
    type TEXT(50) NOT NULL,
    value TEXT(191) NOT NULL,
    unit TEXT(50) NOT NULL,
    reached_at DATETIME NOT NULL
);

CREATE TABLE IF NOT EXISTS recommendations (
    id TEXT PRIMARY KEY NOT NULL,
    url TEXT(2000) NOT NULL,
    title TEXT(191) NOT NULL,
    description TEXT(2000),
    one_click_subscribe BOOLEAN NOT NULL DEFAULT 0,
    featured BOOLEAN NOT NULL DEFAULT 0,
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);

CREATE TABLE IF NOT EXISTS recommendation_click_events (
    id TEXT PRIMARY KEY NOT NULL,
    recommendation_id TEXT(24) NOT NULL REFERENCES recommendations(id) ON DELETE CASCADE,
    member_id TEXT(24),
    created_at DATETIME NOT NULL
);

CREATE TABLE IF NOT EXISTS recommendation_subscribe_events (
    id TEXT PRIMARY KEY NOT NULL,
    recommendation_id TEXT(24) NOT NULL REFERENCES recommendations(id) ON DELETE CASCADE,
    member_id TEXT(24),
    created_at DATETIME NOT NULL
);

CREATE TABLE IF NOT EXISTS members_created_events (
    id TEXT PRIMARY KEY NOT NULL,
    member_id TEXT(24) NOT NULL,
    source TEXT(50) NOT NULL DEFAULT 'admin',
    created_at DATETIME NOT NULL
);
CREATE INDEX IF NOT EXISTS members_created_events_idx ON members_created_events (member_id, source);

CREATE TABLE IF NOT EXISTS members_stripe_customers (
    id TEXT PRIMARY KEY NOT NULL,
    member_id TEXT(24) NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    customer_id TEXT(191) NOT NULL UNIQUE,
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);

CREATE TABLE IF NOT EXISTS subscriptions (
    id TEXT PRIMARY KEY NOT NULL,
    customer_id TEXT(191) NOT NULL,
    subscription_id TEXT(191) NOT NULL UNIQUE,
    status TEXT(50),
    cancel_at_period_end BOOLEAN,
    current_period_end DATETIME,
    start_date DATETIME,
    default_payment_card_last4 TEXT(4),
    stripe_price_id TEXT(255),
    plan_id TEXT(24) REFERENCES products(id),
    plan_interval TEXT(50),
    plan_currency TEXT(50),
    plan_nickname TEXT(50),
    plan_amount INTEGER,
    mrr REAL,
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);

CREATE TABLE IF NOT EXISTS stripe_products (
    id TEXT PRIMARY KEY NOT NULL,
    stripe_product_id TEXT(255) NOT NULL UNIQUE,
    product_id TEXT(24) NOT NULL REFERENCES products(id) ON DELETE CASCADE,
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);

CREATE TABLE IF NOT EXISTS stripe_prices (
    id TEXT PRIMARY KEY NOT NULL,
    stripe_price_id TEXT(255) NOT NULL UNIQUE,
    stripe_product_id TEXT(255) NOT NULL,
    active BOOLEAN NOT NULL DEFAULT 1,
    nickname TEXT(50),
    amount INTEGER,
    currency TEXT(50),
    interval TEXT(50),
    type TEXT(50),
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);
