-- 0034_ecom_paths.sql — Tools → E-commerce: the store's public addresses.
--   ecom_base_path     where the store lives: `shop` → /shop/
--   ecom_product_path  what precedes a product's handle: `shop/products`
INSERT OR IGNORE INTO settings (id,`group`,key,value,type,created_at,updated_at) VALUES
(lower(hex(randomblob(12))),'ecom','ecom_base_path','shop','string',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'ecom','ecom_product_path','shop/products','string',datetime('now'),datetime('now'));
