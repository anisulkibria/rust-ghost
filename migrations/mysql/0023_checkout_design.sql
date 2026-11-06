INSERT IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at)
VALUES (lower(hex(RANDOM_BYTES(12))),'stripe','stripe_checkout_config','{"design":{"customize":false}}','object',UTC_TIMESTAMP(),UTC_TIMESTAMP());
