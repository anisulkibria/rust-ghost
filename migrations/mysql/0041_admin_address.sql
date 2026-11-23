INSERT IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(RANDOM_BYTES(12))),'core','admin_path','vuuut','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'core','admin_default_redirect','true','boolean',UTC_TIMESTAMP(),UTC_TIMESTAMP());
