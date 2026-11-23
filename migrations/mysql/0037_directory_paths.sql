-- Keep existing directory URLs as the defaults for new and upgraded sites.
INSERT IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(RANDOM_BYTES(12))),'directory','directory_base_path','directory','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'directory','directory_listing_path','','string',UTC_TIMESTAMP(),UTC_TIMESTAMP());
