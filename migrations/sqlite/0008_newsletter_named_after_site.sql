-- Name the default newsletter after the site.
--
-- Ghost's fixture calls it "Default Newsletter" and ships with
-- `show_header_name` off, so upstream nobody ever reads that name. This port
-- shows it in the email header, where "Default Newsletter" under the site
-- title is plainly a leftover from a fixture file — a subscriber should see
-- the publication they subscribed to.
--
-- From here on the name is derived from the site title (see
-- `repos::newsletters::default_name`, and `SettingsCache::set`, which keeps
-- the two in step when the title changes). This brings existing installs in
-- line; a newsletter anyone has already named is matched by neither
-- condition and keeps its name.

UPDATE newsletters
SET name = TRIM(COALESCE((SELECT value FROM settings WHERE key = 'title'), '')) || ' Newsletter',
    updated_at = STRFTIME('%Y-%m-%d %H:%M:%S', 'now')
WHERE name = 'Default Newsletter'
  -- An empty or missing title would leave the name as " Newsletter", which
  -- is worse than the fixture name it replaces.
  AND TRIM(COALESCE((SELECT value FROM settings WHERE key = 'title'), '')) <> '';
