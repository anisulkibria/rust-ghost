-- ============================================================================
-- 0024_site_search.sql — Tools → Search and Filtering
-- ============================================================================
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
  id         TEXT PRIMARY KEY,
  query      TEXT NOT NULL,
  results    INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS search_queries_created_idx ON search_queries (created_at);

INSERT OR IGNORE INTO settings (id,`group`,key,value,type,created_at,updated_at) VALUES
(lower(hex(randomblob(12))),'search','search_engine','string','sodo',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'search','search_include_pages','boolean','false',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'search','search_include_members','boolean','true',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'search','search_exclude_tags','string','',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'search','search_full_text','boolean','false',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'search','search_show_authors','boolean','true',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'search','search_show_tags','boolean','true',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'search','search_weight_title','number','8',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'search','search_weight_excerpt','number','4',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'search','search_weight_tags','number','3',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'search','search_weight_body','number','1',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'search','search_order','string','relevance',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'search','search_limit','number','20',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'search','search_synonyms','string','',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'search','search_placeholder','string','',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'search','search_shortcut','boolean','true',datetime('now'),datetime('now')),
(lower(hex(randomblob(12))),'search','search_log','boolean','true',datetime('now'),datetime('now'));
