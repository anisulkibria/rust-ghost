-- 0034_ecom_paths.sql — MySQL twin of migrations/sqlite/0034_ecom_paths.sql.
INSERT IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(RANDOM_BYTES(12))),'ecom','ecom_base_path','shop','string',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'ecom','ecom_product_path','shop/products','string',UTC_TIMESTAMP(),UTC_TIMESTAMP());
