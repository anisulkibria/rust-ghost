CREATE TABLE machine_payment_attempts (
    credential_hash TEXT PRIMARY KEY,
    post_id TEXT NOT NULL,
    resource TEXT NOT NULL,
    protocol TEXT NOT NULL,
    method TEXT NOT NULL,
    amount INTEGER NOT NULL,
    currency TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    reference TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(protocol,reference)
);
CREATE INDEX machine_payment_attempts_time ON machine_payment_attempts(created_at);
CREATE TABLE machine_payment_replay_store (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
INSERT OR IGNORE INTO settings (id,`group`,key,value,type,created_at,updated_at) VALUES
(lower(hex(randomblob(12))),'machine_payments','machine_payments_enabled','false','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'machine_payments','machine_payments_amount','100','number',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'machine_payments','machine_payments_currency','USD','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'machine_payments','machine_payments_secret','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'machine_payments','machine_payments_stripe_profile_id','','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'machine_payments','machine_payments_deposit_address','{}','object',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'machine_payments','machine_payments_x402_enabled','false','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'machine_payments','machine_payments_x402_network','eip155:8453','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'machine_payments','machine_payments_x402_facilitator','https://facilitator.xpay.sh','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'machine_payments','machine_payments_tempo_enabled','false','boolean',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'machine_payments','machine_payments_tempo_testnet','false','boolean',datetime('now'),datetime('now'));
