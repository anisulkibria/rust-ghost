-- Keep existing directory URLs as the defaults for new and upgraded sites.
INSERT OR IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(randomblob(12))),'directory','directory_base_path','directory','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'directory','directory_listing_path','','string',datetime('now'),datetime('now'));
