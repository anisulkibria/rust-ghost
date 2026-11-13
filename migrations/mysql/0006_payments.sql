-- Payments: donation and Stripe checkout settings, the paid-members switch, and
-- machine payments.
--
-- Consolidated from earlier migrations. Each section below is one of them (or the
-- named part of one), unchanged and in the order it was first applied.

-- ============================================================================
-- donation_price_settings  (was 0014_donation_price_settings.sql)
-- ============================================================================

-- The Stripe price a tip lands on. Ghost caches its donation price in the
-- `stripe_prices` table; this port has no such table, so the ids live in
-- settings where SettingsCache can read them without a query.
--
-- `SettingsCache::set` is an UPDATE, not an upsert — an unknown key is a
-- hard error — so every key the checkout writes must exist up front.
INSERT INTO settings (id, `group`, `key`, value, type, created_at, updated_at)
SELECT lower(hex(RANDOM_BYTES(12))), g, k, v, 'string', UTC_TIMESTAMP(), UTC_TIMESTAMP()
FROM (
    SELECT 'members' AS g, 'donations_product_id' AS k, '' AS v UNION ALL
    SELECT 'members',       'donations_price_id',         ''
) AS defaults
WHERE NOT EXISTS (SELECT 1 FROM settings s WHERE s.`key` = defaults.k);

-- ============================================================================
-- paid_members_enabled_setting  (was 0016_paid_members_enabled_setting.sql)
-- ============================================================================

-- `paid_members_enabled` had no row to update.
--
-- Connecting Stripe writes this flag, and `SettingsCache::set` is an UPDATE
-- that errors on an unknown key — so the write silently never landed and the
-- flag stayed false on every install. Portal reads it to decide whether paid
-- membership exists at all, which made `#/portal/signup/monthly`,
-- `#/portal/signup/<tier>/<cadence>`, `#/portal/account/plans`,
-- `#/portal/gift` and `#/portal/support` all refuse to open even with Stripe
-- connected and tiers priced.
--
-- Seeded from the connection that is already there, rather than false: a site
-- that connected Stripe before this migration should not have to reconnect.
INSERT INTO settings (id, `group`, `key`, value, type, created_at, updated_at)
SELECT lower(hex(RANDOM_BYTES(12))), 'members', 'paid_members_enabled',
       CASE WHEN EXISTS (
           SELECT 1 FROM settings
           WHERE `key` IN ('stripe_secret_key', 'stripe_connect_secret_key')
             AND value IS NOT NULL AND trim(value) <> ''
       ) THEN 'true' ELSE 'false' END,
       'boolean', UTC_TIMESTAMP(), UTC_TIMESTAMP()
WHERE NOT EXISTS (SELECT 1 FROM settings WHERE `key` = 'paid_members_enabled');

-- ============================================================================
-- stripe_checkout_setup_error  (was 0017_stripe_checkout_setup_error.sql)
-- ============================================================================

-- Checkout keeps an owner-facing setup diagnostic while Portal receives a
-- message it knows how to translate. SettingsCache only updates known keys.
INSERT INTO settings (id, `group`, `key`, value, type, created_at, updated_at)
SELECT lower(hex(RANDOM_BYTES(12))), 'members', 'stripe_checkout_setup_error', '', 'string', UTC_TIMESTAMP(), UTC_TIMESTAMP()
WHERE NOT EXISTS (SELECT 1 FROM settings WHERE `key` = 'stripe_checkout_setup_error');

-- ============================================================================
-- machine_payments  (was 0021_machine_payments.sql)
-- ============================================================================

CREATE TABLE machine_payment_attempts (
    `credential_hash` VARCHAR(191) PRIMARY KEY,
    `post_id` VARCHAR(1000) NOT NULL,
    `resource` VARCHAR(1000) NOT NULL,
    `protocol` VARCHAR(191) NOT NULL,
    `method` VARCHAR(1000) NOT NULL,
    `amount` BIGINT NOT NULL,
    `currency` VARCHAR(1000) NOT NULL,
    `status` VARCHAR(191) NOT NULL DEFAULT 'pending',
    `reference` VARCHAR(191) ,
    `created_at` VARCHAR(191) NOT NULL,
    `updated_at` VARCHAR(32) NOT NULL,
    UNIQUE(protocol,reference)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX machine_payment_attempts_time ON machine_payment_attempts (`created_at`);
CREATE TABLE machine_payment_replay_store (
    `key` VARCHAR(191) PRIMARY KEY,
    `value` VARCHAR(4000) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
INSERT IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(RANDOM_BYTES(12))),'machine_payments','machine_payments_enabled','false','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'machine_payments','machine_payments_amount','100','number',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'machine_payments','machine_payments_currency','USD','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'machine_payments','machine_payments_secret','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'machine_payments','machine_payments_stripe_profile_id','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'machine_payments','machine_payments_deposit_address','{}','object',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'machine_payments','machine_payments_x402_enabled','false','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'machine_payments','machine_payments_x402_network','eip155:8453','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'machine_payments','machine_payments_x402_facilitator','https://facilitator.xpay.sh','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'machine_payments','machine_payments_tempo_enabled','false','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'machine_payments','machine_payments_tempo_testnet','false','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP());

-- ============================================================================
-- checkout_design  (was 0023_checkout_design.sql)
-- ============================================================================

INSERT IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at)
VALUES (lower(hex(RANDOM_BYTES(12))),'stripe','stripe_checkout_config','{"design":{"customize":false}}','object',UTC_TIMESTAMP(),UTC_TIMESTAMP());
