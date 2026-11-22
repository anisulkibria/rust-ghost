-- ============================================================================
-- 0024_site_search.sql — Tools → Search and Filtering
-- ============================================================================
--
-- The MySQL twin of migrations/sqlite/0024_site_search.sql. VARCHAR, not
-- TEXT, because sqlx `Any` cannot read TEXT back from MySQL.
--
-- One table and seventeen settings rows.
--
-- `search_queries` is the insights log: one row per search a visitor settled
-- on (the visitor script sends it after a pause or a click, not on every
-- keystroke). It holds the query in its normalised form and how many results
-- it found — no address, no cookie, no member id — and rows older than 90
-- days are deleted by the tool itself. A zero in `results` is the useful
-- part: it is a reader asking for something the site does not have.
--
-- Every setting reproduces the search as it was before this tool existed:
-- Ghost's Sodo Search, posts only, members-only posts listed by title and
-- excerpt, authors and tags shown. Applying this changes nothing a visitor
-- sees. `ghost_core::site_search::Settings::read` also treats a missing row
-- as that default.
CREATE TABLE IF NOT EXISTS search_queries (
  id         VARCHAR(24) PRIMARY KEY,
  query      VARCHAR(120) NOT NULL,
  results    INT NOT NULL DEFAULT 0,
  created_at VARCHAR(32) NOT NULL,
  INDEX search_queries_created_idx (created_at)
);

INSERT IGNORE INTO settings (id,`group`,`key`,value,type,created_at,updated_at) VALUES
(lower(hex(RANDOM_BYTES(12))),'search','search_engine','string','sodo',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'search','search_include_pages','boolean','false',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'search','search_include_members','boolean','true',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'search','search_exclude_tags','string','',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'search','search_full_text','boolean','false',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'search','search_show_authors','boolean','true',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'search','search_show_tags','boolean','true',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'search','search_weight_title','number','8',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'search','search_weight_excerpt','number','4',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'search','search_weight_tags','number','3',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'search','search_weight_body','number','1',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'search','search_order','string','relevance',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'search','search_limit','number','20',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'search','search_synonyms','string','',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'search','search_placeholder','string','',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'search','search_shortcut','boolean','true',UTC_TIMESTAMP(),UTC_TIMESTAMP()),
(lower(hex(RANDOM_BYTES(12))),'search','search_log','boolean','true',UTC_TIMESTAMP(),UTC_TIMESTAMP());
