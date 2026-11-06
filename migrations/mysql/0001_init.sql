-- Ghost-rust initial schema.
-- Column names/types mirror Ghost's server/data/schema/schema.js so that a
-- real Ghost SQLite database can be imported via INSERT INTO ... SELECT.
-- PKs are app-generated 24-hex ObjectIds; booleans are 0/1; dates are
-- TEXT 'YYYY-MM-DD HH:mm:ss' UTC.

CREATE TABLE IF NOT EXISTS posts (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `uuid` VARCHAR(36) ,
    `title` VARCHAR(1000) ,
    `slug` VARCHAR(191) NOT NULL,
    `mobiledoc` MEDIUMTEXT ,
    `lexical` MEDIUMTEXT ,
    `html` MEDIUMTEXT ,
    `comment_id` VARCHAR(50) ,
    `plaintext` MEDIUMTEXT ,
    `feature_image` VARCHAR(2000) ,
    `featured` SMALLINT NOT NULL DEFAULT 0,
    `type` VARCHAR(50) NOT NULL DEFAULT 'post',
    `status` VARCHAR(50) NOT NULL DEFAULT 'draft',
    `locale` VARCHAR(6) ,
    `visibility` VARCHAR(50) NOT NULL DEFAULT 'public',
    `email_recipient_filter` MEDIUMTEXT ,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32) ,
    `published_at` VARCHAR(32) ,
    `published_by` VARCHAR(24) ,
    `custom_excerpt` VARCHAR(2000) ,
    `auto_excerpt` VARCHAR(500) ,
    `reading_time` BIGINT UNSIGNED,
    `codeinjection_head` MEDIUMTEXT ,
    `codeinjection_foot` MEDIUMTEXT ,
    `custom_template` VARCHAR(100) ,
    `canonical_url` VARCHAR(2000) ,
    `newsletter_id` VARCHAR(191) ,
    `show_title_and_feature_image` SMALLINT NOT NULL DEFAULT 1
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX posts_uuid_idx ON posts (`uuid`);
CREATE INDEX posts_published_at_idx ON posts (`published_at`);
CREATE INDEX posts_updated_at_idx ON posts (`updated_at`);
CREATE INDEX posts_type_status_idx ON posts (`type`, `status`, `updated_at`);
CREATE UNIQUE INDEX posts_slug_type_unique ON posts (`slug`, `type`);

CREATE TABLE IF NOT EXISTS posts_meta (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `post_id` VARCHAR(191) NOT NULL UNIQUE ,
    `og_image` VARCHAR(2000) ,
    `og_title` VARCHAR(300) ,
    `og_description` VARCHAR(500) ,
    `twitter_image` VARCHAR(2000) ,
    `twitter_title` VARCHAR(300) ,
    `twitter_description` VARCHAR(500) ,
    `meta_title` VARCHAR(300) ,
    `meta_description` VARCHAR(500) ,
    `email_subject` VARCHAR(300) ,
    `email_only` SMALLINT NOT NULL DEFAULT 0,
    `email_preview` VARCHAR(300) ,
    `frontmatter` MEDIUMTEXT ,
    `feature_image_alt` VARCHAR(2000) ,
    `feature_image_caption` VARCHAR(1000)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS users (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `name` VARCHAR(191) NOT NULL,
    `slug` VARCHAR(191) NOT NULL UNIQUE,
    `password` VARCHAR(60) NOT NULL,
    `email` VARCHAR(191) NOT NULL UNIQUE,
    `profile_image` VARCHAR(2000) ,
    `cover_image` VARCHAR(2000) ,
    `bio` VARCHAR(2000) ,
    `website` VARCHAR(2000) ,
    `location` VARCHAR(150) ,
    `facebook` VARCHAR(2000) ,
    `twitter` VARCHAR(2000) ,
    `threads` VARCHAR(191) ,
    `bluesky` VARCHAR(191) ,
    `mastodon` VARCHAR(191) ,
    `tiktok` VARCHAR(191) ,
    `youtube` VARCHAR(191) ,
    `instagram` VARCHAR(191) ,
    `linkedin` VARCHAR(191) ,
    `accessibility` MEDIUMTEXT ,
    `status` VARCHAR(50) NOT NULL DEFAULT 'active',
    `locale` VARCHAR(6) ,
    `visibility` VARCHAR(50) NOT NULL DEFAULT 'public',
    `meta_title` VARCHAR(300) ,
    `meta_description` VARCHAR(500) ,
    `tour` MEDIUMTEXT ,
    `last_seen` VARCHAR(32) ,
    `comment_notifications` SMALLINT NOT NULL DEFAULT 1,
    `free_member_signup_notification` SMALLINT NOT NULL DEFAULT 1,
    `paid_subscription_started_notification` SMALLINT NOT NULL DEFAULT 1,
    `paid_subscription_canceled_notification` SMALLINT NOT NULL DEFAULT 0,
    `mention_notifications` SMALLINT NOT NULL DEFAULT 1,
    `recommendation_notifications` SMALLINT NOT NULL DEFAULT 1,
    `milestone_notifications` SMALLINT NOT NULL DEFAULT 1,
    `donation_notifications` SMALLINT NOT NULL DEFAULT 1,
    `gift_subscription_notifications` SMALLINT NOT NULL DEFAULT 1,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS tags (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `name` VARCHAR(191) NOT NULL,
    `slug` VARCHAR(191) NOT NULL UNIQUE,
    `description` VARCHAR(500) ,
    `feature_image` VARCHAR(2000) ,
    `parent_id` VARCHAR(24) ,
    `visibility` VARCHAR(50) NOT NULL DEFAULT 'public',
    `og_image` VARCHAR(2000) ,
    `og_title` VARCHAR(300) ,
    `og_description` VARCHAR(500) ,
    `twitter_image` VARCHAR(2000) ,
    `twitter_title` VARCHAR(300) ,
    `twitter_description` VARCHAR(500) ,
    `meta_title` VARCHAR(300) ,
    `meta_description` VARCHAR(500) ,
    `codeinjection_head` MEDIUMTEXT ,
    `codeinjection_foot` MEDIUMTEXT ,
    `canonical_url` VARCHAR(2000) ,
    `accent_color` VARCHAR(50) ,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS posts_tags (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `post_id` VARCHAR(191) NOT NULL ,
    `tag_id` VARCHAR(191) NOT NULL ,
    `sort_order` BIGINT UNSIGNED NOT NULL DEFAULT 0
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX posts_tags_post_id_tag_id_idx ON posts_tags (`post_id`, `tag_id`);

CREATE TABLE IF NOT EXISTS posts_authors (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `post_id` VARCHAR(191) NOT NULL ,
    `author_id` VARCHAR(191) NOT NULL ,
    `sort_order` BIGINT UNSIGNED NOT NULL DEFAULT 0
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX posts_authors_post_id_author_id_idx ON posts_authors (`post_id`, `author_id`);

CREATE TABLE IF NOT EXISTS roles (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `name` VARCHAR(50) NOT NULL UNIQUE,
    `description` VARCHAR(2000) ,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS roles_users (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `role_id` VARCHAR(24) NOT NULL,
    `user_id` VARCHAR(24) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS permissions (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `name` VARCHAR(50) NOT NULL UNIQUE,
    `object_type` VARCHAR(50) NOT NULL,
    `action_type` VARCHAR(50) NOT NULL,
    `object_id` VARCHAR(24) ,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS permissions_users (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `user_id` VARCHAR(24) NOT NULL,
    `permission_id` VARCHAR(24) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS permissions_roles (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `role_id` VARCHAR(24) NOT NULL,
    `permission_id` VARCHAR(24) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS settings (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `group` VARCHAR(50) NOT NULL DEFAULT 'core',
    `key` VARCHAR(50) NOT NULL UNIQUE,
    `value` MEDIUMTEXT ,
    `type` VARCHAR(50) NOT NULL DEFAULT 'string',
    `flags` VARCHAR(50) ,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS sessions (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `session_id` VARCHAR(32) NOT NULL UNIQUE,
    `user_id` VARCHAR(24) NOT NULL,
    `session_data` VARCHAR(2000) NOT NULL,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS integrations (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `type` VARCHAR(50) NOT NULL DEFAULT 'custom',
    `name` VARCHAR(191) NOT NULL,
    `slug` VARCHAR(191) NOT NULL UNIQUE,
    `icon_image` VARCHAR(2000) ,
    `description` VARCHAR(2000) ,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS webhooks (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `event` VARCHAR(50) NOT NULL,
    `target_url` VARCHAR(2000) NOT NULL,
    `name` VARCHAR(191) ,
    `secret` VARCHAR(191) ,
    `api_version` VARCHAR(50) NOT NULL DEFAULT 'v2',
    `integration_id` VARCHAR(191) NOT NULL ,
    `last_triggered_at` VARCHAR(32) ,
    `last_triggered_status` VARCHAR(50) ,
    `last_triggered_error` VARCHAR(50) ,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS api_keys (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `type` VARCHAR(50) NOT NULL,
    `secret` VARCHAR(191) NOT NULL UNIQUE,
    `role_id` VARCHAR(24) ,
    `integration_id` VARCHAR(191) ,
    `user_id` VARCHAR(24) ,
    `last_seen_at` VARCHAR(32) ,
    `last_seen_version` VARCHAR(50) ,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS actions (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `resource_id` VARCHAR(24) ,
    `resource_type` VARCHAR(50) NOT NULL,
    `actor_id` VARCHAR(24) NOT NULL,
    `actor_type` VARCHAR(50) NOT NULL,
    `event` VARCHAR(50) NOT NULL,
    `context` VARCHAR(4000) ,
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX actions_resource_idx ON actions (`resource_type`, `resource_id`);

CREATE TABLE IF NOT EXISTS invites (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `role_id` VARCHAR(24) NOT NULL,
    `token` VARCHAR(191) NOT NULL UNIQUE,
    `expires` BIGINT NOT NULL,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS brute (
    `key` VARCHAR(191) PRIMARY KEY NOT NULL,
    `firstRequest` BIGINT ,
    `lastRequest` BIGINT ,
    `lifetime` BIGINT ,
    `count` BIGINT
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS tokens (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `token` VARCHAR(64) NOT NULL,
    `uuid` VARCHAR(36) UNIQUE,
    `data` VARCHAR(2000) ,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32) ,
    `first_used_at` VARCHAR(32) ,
    `used_count` BIGINT UNSIGNED NOT NULL DEFAULT 0,
    `otc_used_count` BIGINT UNSIGNED NOT NULL DEFAULT 0
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX tokens_token_idx ON tokens (`token`);

CREATE TABLE IF NOT EXISTS snippets (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `name` VARCHAR(191) NOT NULL UNIQUE,
    `mobiledoc` MEDIUMTEXT NOT NULL,
    `lexical` MEDIUMTEXT ,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS custom_theme_settings (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `theme` VARCHAR(191) NOT NULL,
    `key` VARCHAR(191) NOT NULL,
    `value` MEDIUMTEXT
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE UNIQUE INDEX custom_theme_settings_theme_key_unique ON custom_theme_settings (`theme`, `key`);

-- ---------------------------------------------------------------------------
-- Deferred-phase tables (members/newsletters/emails/comments/offers/tiers).
-- Schema-only so future phases and Ghost DB imports have somewhere to land.
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS members (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `uuid` VARCHAR(36) UNIQUE,
    `transient_id` VARCHAR(191) NOT NULL UNIQUE,
    `email` VARCHAR(191) NOT NULL UNIQUE,
    `status` VARCHAR(50) NOT NULL DEFAULT 'free',
    `name` VARCHAR(191) ,
    `expertise` VARCHAR(50) ,
    `note` VARCHAR(2000) ,
    `geolocation` VARCHAR(2000) ,
    `enable_comment_notifications` SMALLINT NOT NULL DEFAULT 1,
    `enable_updates_and_announcements` SMALLINT ,
    `email_count` BIGINT UNSIGNED NOT NULL DEFAULT 0,
    `email_opened_count` BIGINT UNSIGNED NOT NULL DEFAULT 0,
    `email_open_rate` BIGINT UNSIGNED,
    `email_disabled` SMALLINT NOT NULL DEFAULT 0,
    `last_seen_at` VARCHAR(32) ,
    `last_commented_at` VARCHAR(32) ,
    `commenting` VARCHAR(4000) ,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX members_created_at_idx ON members (`created_at`, `id`);
CREATE INDEX members_email_disabled_idx ON members (`email_disabled`);

CREATE TABLE IF NOT EXISTS products (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `name` VARCHAR(191) NOT NULL,
    `slug` VARCHAR(191) NOT NULL UNIQUE,
    `active` SMALLINT NOT NULL DEFAULT 1,
    `welcome_page_url` VARCHAR(2000) ,
    `visibility` VARCHAR(50) NOT NULL DEFAULT 'none',
    `trial_days` BIGINT UNSIGNED NOT NULL DEFAULT 0,
    `description` VARCHAR(191) ,
    `type` VARCHAR(50) NOT NULL DEFAULT 'paid',
    `currency` VARCHAR(50) ,
    `monthly_price` BIGINT UNSIGNED,
    `yearly_price` BIGINT UNSIGNED,
    `monthly_price_id` VARCHAR(24) ,
    `yearly_price_id` VARCHAR(24) ,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS members_products (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `member_id` VARCHAR(191) NOT NULL ,
    `product_id` VARCHAR(191) NOT NULL ,
    `sort_order` BIGINT UNSIGNED NOT NULL DEFAULT 0,
    `expiry_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS posts_products (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `post_id` VARCHAR(191) NOT NULL ,
    `product_id` VARCHAR(191) NOT NULL ,
    `sort_order` BIGINT UNSIGNED NOT NULL DEFAULT 0
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS benefits (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `name` VARCHAR(191) NOT NULL,
    `slug` VARCHAR(191) NOT NULL UNIQUE,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS products_benefits (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `product_id` VARCHAR(191) NOT NULL ,
    `benefit_id` VARCHAR(191) NOT NULL ,
    `sort_order` BIGINT UNSIGNED NOT NULL DEFAULT 0
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS offers (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `active` SMALLINT NOT NULL DEFAULT 0,
    `name` VARCHAR(191) NOT NULL UNIQUE,
    `code` VARCHAR(191) NOT NULL UNIQUE,
    `product_id` VARCHAR(191) NOT NULL ,
    `stripe_coupon_id` VARCHAR(255) UNIQUE,
    `interval` VARCHAR(50) ,
    `currency` VARCHAR(50) ,
    `discount_type` VARCHAR(50) ,
    `discount_amount` BIGINT ,
    `duration` VARCHAR(50) ,
    `duration_in_months` BIGINT ,
    `portal_title` VARCHAR(191) ,
    `portal_description` VARCHAR(2000) ,
    `redemption_type` VARCHAR(50) ,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS newsletters (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `uuid` VARCHAR(36) ,
    `name` VARCHAR(191) NOT NULL UNIQUE,
    `description` VARCHAR(2000) ,
    `feedback_enabled` SMALLINT NOT NULL DEFAULT 0,
    `slug` VARCHAR(191) NOT NULL UNIQUE,
    `sender_name` VARCHAR(191) ,
    `sender_email` VARCHAR(191) ,
    `sender_reply_to` VARCHAR(191) NOT NULL DEFAULT 'newsletter',
    `status` VARCHAR(50) NOT NULL DEFAULT 'active',
    `visibility` VARCHAR(50) NOT NULL DEFAULT 'members',
    `subscribe_on_signup` SMALLINT NOT NULL DEFAULT 1,
    `sort_order` BIGINT UNSIGNED NOT NULL DEFAULT 0,
    `header_image` VARCHAR(2000) ,
    `show_header_icon` SMALLINT NOT NULL DEFAULT 1,
    `show_header_title` SMALLINT NOT NULL DEFAULT 1,
    `show_excerpt` SMALLINT NOT NULL DEFAULT 0,
    `show_feature_image` SMALLINT NOT NULL DEFAULT 1,
    `footer_content` VARCHAR(4000) ,
    `show_badge` SMALLINT NOT NULL DEFAULT 1,
    `show_header_name` SMALLINT NOT NULL DEFAULT 1,
    `show_post_title_section` SMALLINT NOT NULL DEFAULT 1,
    `show_comment_cta` SMALLINT NOT NULL DEFAULT 1,
    `show_subscription_details` SMALLINT NOT NULL DEFAULT 0,
    `show_latest_posts` SMALLINT NOT NULL DEFAULT 0,
    `show_share_button` SMALLINT NOT NULL DEFAULT 0,
    `background_color` VARCHAR(50) NOT NULL DEFAULT 'light',
    `post_title_color` VARCHAR(50) ,
    `header_background_color` VARCHAR(50) NOT NULL DEFAULT 'transparent',
    `section_title_color` VARCHAR(50) ,
    `divider_color` VARCHAR(50) ,
    `button_color` VARCHAR(50) DEFAULT 'accent',
    `link_color` VARCHAR(50) DEFAULT 'accent',
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS members_newsletters (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `member_id` VARCHAR(191) NOT NULL ,
    `newsletter_id` VARCHAR(191) NOT NULL ,
    `status` VARCHAR(50) NOT NULL DEFAULT 'confirmed'
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS emails (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `post_id` VARCHAR(24) NOT NULL UNIQUE,
    `status` VARCHAR(50) NOT NULL DEFAULT 'pending',
    `recipient_filter` MEDIUMTEXT ,
    `error` VARCHAR(2000) ,
    `error_data` MEDIUMTEXT ,
    `email_count` BIGINT UNSIGNED NOT NULL DEFAULT 0,
    `delivered_count` BIGINT UNSIGNED NOT NULL DEFAULT 0,
    `opened_count` BIGINT UNSIGNED NOT NULL DEFAULT 0,
    `failed_count` BIGINT UNSIGNED NOT NULL DEFAULT 0,
    `subject` VARCHAR(300) ,
    `from` VARCHAR(2000) ,
    `reply_to` VARCHAR(2000) ,
    `html` MEDIUMTEXT ,
    `plaintext` MEDIUMTEXT ,
    `source` MEDIUMTEXT ,
    `track_opens` SMALLINT NOT NULL DEFAULT 0,
    `track_clicks` SMALLINT NOT NULL DEFAULT 0,
    `feedback_enabled` SMALLINT NOT NULL DEFAULT 0,
    `submitted_at` VARCHAR(32) ,
    `newsletter_id` VARCHAR(191) ,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS email_batches (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `email_id` VARCHAR(191) NOT NULL ,
    `provider_id` VARCHAR(191) ,
    `status` VARCHAR(50) NOT NULL DEFAULT 'submitted',
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS email_recipients (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `email_id` VARCHAR(191) NOT NULL ,
    `member_id` VARCHAR(24) ,
    `batch_id` VARCHAR(24) ,
    `status` VARCHAR(50) NOT NULL DEFAULT 'pending',
    `delivered_at` VARCHAR(32) ,
    `opened_at` VARCHAR(32) ,
    `failed_at` VARCHAR(32) ,
    `error_code` BIGINT ,
    `error` VARCHAR(2000) ,
    `email_address` VARCHAR(191) ,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX email_recipients_email_member_idx ON email_recipients (`email_id`, `member_id`);

CREATE TABLE IF NOT EXISTS email_recipient_failures (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `email_id` VARCHAR(191) NOT NULL ,
    `email_recipient_id` VARCHAR(24) ,
    `member_id` VARCHAR(24) ,
    `message` VARCHAR(2000) ,
    `failed_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS comments (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `post_id` VARCHAR(191) NOT NULL ,
    `member_id` VARCHAR(191) ,
    `parent_id` VARCHAR(191) ,
    `in_reply_to_id` VARCHAR(191) ,
    `status` VARCHAR(50) NOT NULL DEFAULT 'published',
    `html` MEDIUMTEXT ,
    `edited_at` VARCHAR(32) ,
    `pinned_at` VARCHAR(32) ,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX comments_post_id_idx ON comments (`post_id`, `status`);

CREATE TABLE IF NOT EXISTS comment_likes (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `comment_id` VARCHAR(191) NOT NULL ,
    `member_id` VARCHAR(191) NOT NULL ,
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE UNIQUE INDEX comment_likes_unique ON comment_likes (`comment_id`, `member_id`);

CREATE TABLE IF NOT EXISTS comment_reports (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `comment_id` VARCHAR(191) NOT NULL ,
    `member_id` VARCHAR(191) NOT NULL ,
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE UNIQUE INDEX comment_reports_unique ON comment_reports (`comment_id`, `member_id`);

CREATE TABLE IF NOT EXISTS labels (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `name` VARCHAR(191) NOT NULL,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE UNIQUE INDEX labels_name_unique ON labels (`name`);

CREATE TABLE IF NOT EXISTS members_labels (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `label_id` VARCHAR(191) NOT NULL ,
    `member_id` VARCHAR(191) NOT NULL ,
    `sort_order` BIGINT UNSIGNED NOT NULL DEFAULT 0
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE UNIQUE INDEX members_labels_unique ON members_labels (`member_id`, `label_id`);

CREATE TABLE IF NOT EXISTS mobiledoc_revisions (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `post_id` VARCHAR(24) NOT NULL,
    `mobiledoc` MEDIUMTEXT ,
    `created_at_ts` BIGINT NOT NULL,
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX mobiledoc_revisions_post_idx ON mobiledoc_revisions (`post_id`);

CREATE TABLE IF NOT EXISTS post_revisions (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `post_id` VARCHAR(24) NOT NULL,
    `lexical` MEDIUMTEXT ,
    `created_at_ts` BIGINT NOT NULL,
    `created_at` VARCHAR(32) NOT NULL,
    `author_id` VARCHAR(191) ,
    `reason` VARCHAR(50) ,
    `feature_image` VARCHAR(2000) ,
    `feature_image_caption` VARCHAR(1000)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX post_revisions_post_idx ON post_revisions (`post_id`);

CREATE TABLE IF NOT EXISTS milestones (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `type` VARCHAR(50) NOT NULL,
    `value` VARCHAR(191) NOT NULL,
    `unit` VARCHAR(50) NOT NULL,
    `reached_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS recommendations (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `url` VARCHAR(2000) NOT NULL,
    `title` VARCHAR(191) NOT NULL,
    `description` VARCHAR(2000) ,
    `one_click_subscribe` SMALLINT NOT NULL DEFAULT 0,
    `featured` SMALLINT NOT NULL DEFAULT 0,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS recommendation_click_events (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `recommendation_id` VARCHAR(191) NOT NULL ,
    `member_id` VARCHAR(24) ,
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS recommendation_subscribe_events (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `recommendation_id` VARCHAR(191) NOT NULL ,
    `member_id` VARCHAR(24) ,
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS members_created_events (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `member_id` VARCHAR(24) NOT NULL,
    `source` VARCHAR(50) NOT NULL DEFAULT 'admin',
    `created_at` VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX members_created_events_idx ON members_created_events (`member_id`, `source`);

CREATE TABLE IF NOT EXISTS members_stripe_customers (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `member_id` VARCHAR(191) NOT NULL ,
    `customer_id` VARCHAR(191) NOT NULL UNIQUE,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS subscriptions (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `customer_id` VARCHAR(191) NOT NULL,
    `subscription_id` VARCHAR(191) NOT NULL UNIQUE,
    `status` VARCHAR(50) ,
    `cancel_at_period_end` SMALLINT ,
    `current_period_end` VARCHAR(32) ,
    `start_date` VARCHAR(32) ,
    `default_payment_card_last4` VARCHAR(4) ,
    `stripe_price_id` VARCHAR(255) ,
    `plan_id` VARCHAR(191) ,
    `plan_interval` VARCHAR(50) ,
    `plan_currency` VARCHAR(50) ,
    `plan_nickname` VARCHAR(50) ,
    `plan_amount` BIGINT ,
    `mrr` DOUBLE ,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS stripe_products (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `stripe_product_id` VARCHAR(255) NOT NULL UNIQUE,
    `product_id` VARCHAR(191) NOT NULL ,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS stripe_prices (
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `stripe_price_id` VARCHAR(255) NOT NULL UNIQUE,
    `stripe_product_id` VARCHAR(255) NOT NULL,
    `active` SMALLINT NOT NULL DEFAULT 1,
    `nickname` VARCHAR(50) ,
    `amount` BIGINT ,
    `currency` VARCHAR(50) ,
    `interval` VARCHAR(50) ,
    `type` VARCHAR(50) ,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Foreign keys are added after the tables they point at.
-- SQLite resolves a reference lazily and accepts a forward one;
-- MySQL requires the referenced table to exist already.
ALTER TABLE posts ADD CONSTRAINT posts_newsletter_id_fk FOREIGN KEY (`newsletter_id`) REFERENCES newsletters(`id`);
ALTER TABLE posts_meta ADD CONSTRAINT posts_meta_post_id_fk FOREIGN KEY (`post_id`) REFERENCES posts(`id`) ON DELETE CASCADE;
ALTER TABLE posts_tags ADD CONSTRAINT posts_tags_post_id_fk FOREIGN KEY (`post_id`) REFERENCES posts(`id`) ON DELETE CASCADE;
ALTER TABLE posts_tags ADD CONSTRAINT posts_tags_tag_id_fk2 FOREIGN KEY (`tag_id`) REFERENCES tags(`id`) ON DELETE CASCADE;
ALTER TABLE posts_authors ADD CONSTRAINT posts_authors_post_id_fk FOREIGN KEY (`post_id`) REFERENCES posts(`id`) ON DELETE CASCADE;
ALTER TABLE posts_authors ADD CONSTRAINT posts_authors_author_id_fk2 FOREIGN KEY (`author_id`) REFERENCES users(`id`) ON DELETE CASCADE;
ALTER TABLE webhooks ADD CONSTRAINT webhooks_integration_id_fk FOREIGN KEY (`integration_id`) REFERENCES integrations(`id`) ON DELETE CASCADE;
ALTER TABLE api_keys ADD CONSTRAINT api_keys_integration_id_fk FOREIGN KEY (`integration_id`) REFERENCES integrations(`id`) ON DELETE CASCADE;
ALTER TABLE members_products ADD CONSTRAINT members_products_member_id_fk FOREIGN KEY (`member_id`) REFERENCES members(`id`) ON DELETE CASCADE;
ALTER TABLE members_products ADD CONSTRAINT members_products_product_id_fk2 FOREIGN KEY (`product_id`) REFERENCES products(`id`) ON DELETE CASCADE;
ALTER TABLE posts_products ADD CONSTRAINT posts_products_post_id_fk FOREIGN KEY (`post_id`) REFERENCES posts(`id`) ON DELETE CASCADE;
ALTER TABLE posts_products ADD CONSTRAINT posts_products_product_id_fk2 FOREIGN KEY (`product_id`) REFERENCES products(`id`) ON DELETE CASCADE;
ALTER TABLE products_benefits ADD CONSTRAINT products_benefits_product_id_fk FOREIGN KEY (`product_id`) REFERENCES products(`id`) ON DELETE CASCADE;
ALTER TABLE products_benefits ADD CONSTRAINT products_benefits_benefit_id_fk2 FOREIGN KEY (`benefit_id`) REFERENCES benefits(`id`) ON DELETE CASCADE;
ALTER TABLE offers ADD CONSTRAINT offers_product_id_fk FOREIGN KEY (`product_id`) REFERENCES products(`id`) ON DELETE CASCADE;
ALTER TABLE members_newsletters ADD CONSTRAINT members_newsletters_member_id_fk FOREIGN KEY (`member_id`) REFERENCES members(`id`) ON DELETE CASCADE;
ALTER TABLE members_newsletters ADD CONSTRAINT members_newsletters_newsletter_id_fk2 FOREIGN KEY (`newsletter_id`) REFERENCES newsletters(`id`) ON DELETE CASCADE;
ALTER TABLE emails ADD CONSTRAINT emails_newsletter_id_fk FOREIGN KEY (`newsletter_id`) REFERENCES newsletters(`id`);
ALTER TABLE email_batches ADD CONSTRAINT email_batches_email_id_fk FOREIGN KEY (`email_id`) REFERENCES emails(`id`) ON DELETE CASCADE;
ALTER TABLE email_recipients ADD CONSTRAINT email_recipients_email_id_fk FOREIGN KEY (`email_id`) REFERENCES emails(`id`) ON DELETE CASCADE;
ALTER TABLE email_recipient_failures ADD CONSTRAINT email_recipient_failures_email_id_fk FOREIGN KEY (`email_id`) REFERENCES emails(`id`) ON DELETE CASCADE;
ALTER TABLE comments ADD CONSTRAINT comments_post_id_fk FOREIGN KEY (`post_id`) REFERENCES posts(`id`) ON DELETE CASCADE;
ALTER TABLE comments ADD CONSTRAINT comments_member_id_fk2 FOREIGN KEY (`member_id`) REFERENCES members(`id`) ON DELETE SET NULL;
ALTER TABLE comments ADD CONSTRAINT comments_parent_id_fk3 FOREIGN KEY (`parent_id`) REFERENCES comments(`id`) ON DELETE CASCADE;
ALTER TABLE comments ADD CONSTRAINT comments_in_reply_to_id_fk4 FOREIGN KEY (`in_reply_to_id`) REFERENCES comments(`id`) ON DELETE SET NULL;
ALTER TABLE comment_likes ADD CONSTRAINT comment_likes_comment_id_fk FOREIGN KEY (`comment_id`) REFERENCES comments(`id`) ON DELETE CASCADE;
ALTER TABLE comment_likes ADD CONSTRAINT comment_likes_member_id_fk2 FOREIGN KEY (`member_id`) REFERENCES members(`id`) ON DELETE CASCADE;
ALTER TABLE comment_reports ADD CONSTRAINT comment_reports_comment_id_fk FOREIGN KEY (`comment_id`) REFERENCES comments(`id`) ON DELETE CASCADE;
ALTER TABLE comment_reports ADD CONSTRAINT comment_reports_member_id_fk2 FOREIGN KEY (`member_id`) REFERENCES members(`id`) ON DELETE CASCADE;
ALTER TABLE members_labels ADD CONSTRAINT members_labels_label_id_fk FOREIGN KEY (`label_id`) REFERENCES labels(`id`) ON DELETE CASCADE;
ALTER TABLE members_labels ADD CONSTRAINT members_labels_member_id_fk2 FOREIGN KEY (`member_id`) REFERENCES members(`id`) ON DELETE CASCADE;
ALTER TABLE post_revisions ADD CONSTRAINT post_revisions_author_id_fk FOREIGN KEY (`author_id`) REFERENCES users(`id`);
ALTER TABLE recommendation_click_events ADD CONSTRAINT recommendation_click_events_recommendation_id_fk FOREIGN KEY (`recommendation_id`) REFERENCES recommendations(`id`) ON DELETE CASCADE;
ALTER TABLE recommendation_subscribe_events ADD CONSTRAINT recommendation_subscribe_events_recommendation_id_fk FOREIGN KEY (`recommendation_id`) REFERENCES recommendations(`id`) ON DELETE CASCADE;
ALTER TABLE members_stripe_customers ADD CONSTRAINT members_stripe_customers_member_id_fk FOREIGN KEY (`member_id`) REFERENCES members(`id`) ON DELETE CASCADE;
ALTER TABLE subscriptions ADD CONSTRAINT subscriptions_plan_id_fk FOREIGN KEY (`plan_id`) REFERENCES products(`id`);
ALTER TABLE stripe_products ADD CONSTRAINT stripe_products_product_id_fk FOREIGN KEY (`product_id`) REFERENCES products(`id`) ON DELETE CASCADE;
