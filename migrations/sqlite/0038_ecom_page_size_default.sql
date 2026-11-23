-- Replace the original seed value and preserve page sizes saved by merchants.
UPDATE settings SET value = '10'
WHERE `key` = 'ecom_per_page' AND value = '24' AND created_at = updated_at;
