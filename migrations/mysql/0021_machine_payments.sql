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
