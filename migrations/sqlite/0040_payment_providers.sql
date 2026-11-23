CREATE TABLE IF NOT EXISTS ecom_payment_products (
    product_id TEXT PRIMARY KEY NOT NULL,
    polar_product TEXT NOT NULL DEFAULT '',
    gumroad_product TEXT NOT NULL DEFAULT '',
    FOREIGN KEY (product_id) REFERENCES ecom_products(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS ecom_hosted_payments (
    order_id TEXT PRIMARY KEY NOT NULL,
    provider VARCHAR(12) NOT NULL,
    product_id TEXT NOT NULL,
    checkout_id TEXT NOT NULL DEFAULT '',
    checkout_url TEXT NOT NULL DEFAULT '',
    receipt_id TEXT NULL,
    UNIQUE(provider, receipt_id),
    FOREIGN KEY (order_id) REFERENCES ecom_orders(id) ON DELETE CASCADE
);

INSERT OR IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(randomblob(12))),'payments','payments_polar_api_key','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'payments','payments_polar_webhook_secret','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'payments','payments_polar_sandbox','true','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'payments','payments_polar_enabled','false','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'payments','payments_gumroad_api_key','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'payments','payments_gumroad_enabled','false','boolean',datetime('now'),datetime('now'));
