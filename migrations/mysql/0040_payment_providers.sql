CREATE TABLE IF NOT EXISTS ecom_payment_products (
    product_id VARCHAR(24) PRIMARY KEY NOT NULL,
    polar_product VARCHAR(191) NOT NULL DEFAULT '',
    gumroad_product VARCHAR(191) NOT NULL DEFAULT '',
    FOREIGN KEY (product_id) REFERENCES ecom_products(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS ecom_hosted_payments (
    order_id VARCHAR(24) PRIMARY KEY NOT NULL,
    provider VARCHAR(12) NOT NULL,
    product_id VARCHAR(191) NOT NULL,
    checkout_id VARCHAR(191) NOT NULL DEFAULT '',
    checkout_url VARCHAR(2083) NOT NULL DEFAULT '',
    receipt_id VARCHAR(191) NULL,
    UNIQUE(provider, receipt_id),
    FOREIGN KEY (order_id) REFERENCES ecom_orders(id) ON DELETE CASCADE
);

INSERT IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(RANDOM_BYTES(12))),'payments','payments_polar_api_key','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'payments','payments_polar_webhook_secret','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'payments','payments_polar_sandbox','true','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'payments','payments_polar_enabled','false','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'payments','payments_gumroad_api_key','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'payments','payments_gumroad_enabled','false','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP());
