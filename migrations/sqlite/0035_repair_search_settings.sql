-- 0024 inserted the setting type into value and the default into type.
-- Match the malformed seed rows exactly so saved preferences are preserved.
UPDATE settings
SET value = type,
    type = CASE
        WHEN `key` IN ('search_include_pages', 'search_include_members', 'search_full_text',
                       'search_show_authors', 'search_show_tags', 'search_shortcut', 'search_log')
            THEN 'boolean'
        WHEN `key` IN ('search_weight_title', 'search_weight_excerpt', 'search_weight_tags',
                       'search_weight_body', 'search_limit')
            THEN 'number'
        ELSE 'string'
    END
WHERE `group` = 'search'
  AND (`key`, value, type) IN (
      ('search_engine', 'string', 'sodo'),
      ('search_include_pages', 'boolean', 'false'),
      ('search_include_members', 'boolean', 'true'),
      ('search_exclude_tags', 'string', ''),
      ('search_full_text', 'boolean', 'false'),
      ('search_show_authors', 'boolean', 'true'),
      ('search_show_tags', 'boolean', 'true'),
      ('search_weight_title', 'number', '8'),
      ('search_weight_excerpt', 'number', '4'),
      ('search_weight_tags', 'number', '3'),
      ('search_weight_body', 'number', '1'),
      ('search_order', 'string', 'relevance'),
      ('search_limit', 'number', '20'),
      ('search_synonyms', 'string', ''),
      ('search_placeholder', 'string', ''),
      ('search_shortcut', 'boolean', 'true'),
      ('search_log', 'boolean', 'true')
  );
