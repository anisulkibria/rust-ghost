INSERT OR IGNORE INTO settings (id,`group`,key,value,type,created_at,updated_at)
VALUES (lower(hex(randomblob(12))),'stripe','stripe_checkout_config','{"design":{"customize":false}}','object',datetime('now'),datetime('now'));
