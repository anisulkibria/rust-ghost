-- ============================================================================
-- 0014_multilingual.sql — publishing in more than one language
-- ============================================================================
--
-- The port already had `posts.locale` and `users.locale` from the initial
-- schema (Ghost carries both and reads neither). Nothing wrote them and
-- nothing read them, so a site had exactly one language: the `locale`
-- setting, which chose which `<theme>/locales/*.json` to load. That is
-- internationalisation — one site, one language, translated theme strings.
--
-- What was missing to make it *multilingual* is here: a way to say which
-- language an entry is in, a way to say that two entries are the same thing
-- in different languages, and a language for a tag.
--
-- ---------------------------------------------------------------------------
-- posts.translation_group — "these are the same entry"
-- ---------------------------------------------------------------------------
--
-- Every translation of one entry shares a group id; an entry with no
-- translations has NULL. A column rather than a join table because the
-- question asked of it is always "what else is in this group", never
-- "what groups is this in" — an entry is one thing in one language, so the
-- relationship is genuinely single-valued and a join table would only add a
-- second row to keep consistent.
--
-- The id is minted from the first entry that gets a sibling, so it is an
-- ordinary 24-character ObjectID like every other id here. Nothing reads it
-- as a reference, so there is no foreign key: a group is whatever rows carry
-- the value, and deleting the last sibling leaves a harmless orphan group of
-- one rather than a dangling pointer.
--
-- ---------------------------------------------------------------------------
-- posts.locale — NULL means the default language
-- ---------------------------------------------------------------------------
--
-- Not backfilled, deliberately. The default language is the `locale`
-- *setting*, which SQL here cannot read, and writing today's value into
-- every row would freeze it: changing the publication language later would
-- silently relabel every existing post as a translation into the old one.
-- So NULL is read as "the site's default language" everywhere
-- (`ghost_core::locales`), and every existing post stays correct through a
-- change of default.
--
-- ---------------------------------------------------------------------------
-- tags.locale — NULL means every language
-- ---------------------------------------------------------------------------
--
-- A tag with no locale is shown in all of them, which is what a site wants
-- for something like `#featured` or a brand name. A tag with a locale
-- appears only in that language's archives and only in that language's tag
-- pickers — the third of the three issues this migration exists for.
--
-- ---------------------------------------------------------------------------
-- The settings rows
-- ---------------------------------------------------------------------------
--
-- `SettingsCache::set` only ever UPDATEs and refuses a key with no row, so a
-- settings screen for a brand-new key has a Save button that always fails.
-- These are the rows behind Settings → Languages.
--
--   locales_enabled               []    Languages besides the default. Empty
--                                       means a single-language site, which
--                                       is what every existing install is —
--                                       so applying this migration changes
--                                       nothing a visitor can see.
--   navigation_locales            {}    locale → navigation array, for the
--   secondary_navigation_locales  {}    languages that want their own menu.
--                                       A language absent from the map falls
--                                       back to the site-wide `navigation`,
--                                       so adding a language does not empty
--                                       its menu.
--   locale_hreflang             true    Emit the `rel=alternate hreflang`
--                                       block. On, because it is the whole
--                                       point of publishing translations: it
--                                       is how a search engine learns that
--                                       two URLs are one page in two
--                                       languages rather than duplicates.

ALTER TABLE posts ADD COLUMN translation_group VARCHAR(24) NULL;
CREATE INDEX posts_translation_group_idx ON posts (translation_group);
-- The locale column already exists; this is what makes "every German post"
-- cheap enough to run on an archive page.
CREATE INDEX posts_locale_idx ON posts (locale);

ALTER TABLE tags ADD COLUMN locale VARCHAR(6) NULL;

INSERT INTO settings (id, `group`, `key`, value, type, created_at, updated_at)
SELECT lower(hex(RANDOM_BYTES(12))), 'multilingual', k, v, t, UTC_TIMESTAMP(), UTC_TIMESTAMP()
FROM (
    SELECT 'locales_enabled'              AS k, '[]'   AS v, 'array'   AS t UNION ALL
    SELECT 'navigation_locales',                 '{}',       'object'        UNION ALL
    SELECT 'secondary_navigation_locales',       '{}',       'object'        UNION ALL
    SELECT 'locale_hreflang',                    'true',     'boolean'
) AS defaults
WHERE NOT EXISTS (SELECT 1 FROM settings s WHERE s.`key` = defaults.k);
