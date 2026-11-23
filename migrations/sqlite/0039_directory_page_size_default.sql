-- Replace the original seed value and preserve page sizes saved by staff.
UPDATE settings SET value = '10'
WHERE `key` = 'directory_per_page' AND value = '24' AND created_at = updated_at;
