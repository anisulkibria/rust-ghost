INSERT OR IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(randomblob(12))),'core','admin_path','vuuut','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'core','admin_default_redirect','true','boolean',datetime('now'),datetime('now'));
