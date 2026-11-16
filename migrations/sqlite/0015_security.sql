-- ============================================================================
-- 0015_security.sql — the Security module's settings
-- ============================================================================
--
-- The module has **no table of its own**, and this time that is not a
-- preference but the point. Everything it reports already exists and already
-- has an owner: `sessions` is who is signed in, `brute` is who is locked out,
-- `api_keys` and `webhooks` are the ways in that are not a password, `invites`
-- is who has been offered one, `users` is who there is, and `actions` is the
-- audit log. A security report holding its own copy of any of that would be a
-- second answer to a question the database already answers, and the two would
-- disagree the first time somebody changed something by another route — at
-- which point the report would be telling an administrator that the site is
-- safe on the strength of a stale row. See
-- crates/ghost-db/src/repos/security.rs.
--
-- What is here is the eight `settings` rows the module writes. They are rows
-- rather than absent keys because `SettingsCache::set` only ever UPDATEs: a
-- key with no row is refused as "unknown setting", so a site that upgraded
-- into this module would show a settings page whose Save button always
-- failed. The same keys are in crates/ghost-db/assets/default-settings.json
-- for a brand-new install; this file is what carries them onto a database
-- that already exists.
--
-- `require_email_mfa` is not here: it exists already. Settings → Security is
-- simply the first screen that offers it, and the Security report the first
-- thing that says what it is worth.
--
-- ----------------------------------------------------------------------------
-- These defaults are not all "whatever the site did before"
-- ----------------------------------------------------------------------------
--
-- The SEO module's migration makes a point of every default reproducing the
-- prior behaviour, so that installing it changes nothing. That rule is right
-- for a module whose settings are editorial preferences and wrong for one
-- whose settings are defences: a hardening switch that ships off is hardening
-- nobody has. The line drawn here is **can this break a working site**, and
-- the reasoning for each is in ghost_core::security's module doc. In short:
--
--   security_headers                     true   `nosniff`, `SAMEORIGIN`,
--                                               `frame-ancestors 'self'`, a
--                                               referrer policy and `no-store`
--                                               on admin responses. The admin
--                                               is never framed by another
--                                               origin and never relies on
--                                               content sniffing, so there is
--                                               no configuration in which
--                                               these break it. CHANGES
--                                               BEHAVIOUR, safely.
--   security_admin_referrer_policy  same-origin An admin URL carries post and
--                                               member ids in its path. Only
--                                               values that never send a path
--                                               off-origin are offered.
--   security_hsts                        false  A browser holds the promise
--                                               for two years and cannot be
--                                               told to forget it quickly. A
--                                               site that later loses its
--                                               certificate is unreachable,
--                                               not merely insecure. Somebody
--                                               has to choose this.
--   security_trust_proxy                 true   Believe `X-Forwarded-For`.
--                                               This IS the old behaviour and
--                                               it is the one case where the
--                                               permissive default wins: off,
--                                               on a proxied site, every
--                                               visitor shares one rate-limit
--                                               budget and the site locks
--                                               itself out — and nothing
--                                               inside the process can tell
--                                               whether there is a proxy. The
--                                               report flags it as critical
--                                               instead, because served
--                                               directly it makes every
--                                               brute-force budget in the
--                                               application advisory.
--   security_private_fetch               true   Refuse to fetch a URL that
--                                               resolves inside this server's
--                                               own network. The only thing it
--                                               stops is a bookmark or embed
--                                               card reading localhost or a
--                                               cloud metadata service, which
--                                               the webmention receiver has
--                                               refused since it shipped.
--                                               CHANGES BEHAVIOUR, safely.
--   security_session_days                '7'    How long a staff session may
--                                               live. 7 is the `Max-Age` the
--                                               session cookie has always
--                                               carried, so a browser was
--                                               already dropping it on this
--                                               schedule; what changes is that
--                                               the row behind it expires too,
--                                               so an id copied out of the
--                                               database stops working when
--                                               the cookie would have.
--   security_session_idle_minutes        '0'    No idle timeout, which is what
--                                               the site did. On, it signs
--                                               somebody out of a tab they
--                                               left open — a real improvement
--                                               and somebody's choice.
--   security_min_password_length        '10'    Ghost's own minimum, and what
--                                               every password path in this
--                                               application already enforces.
--   security_revoke_on_password_change   true   Sign an account out of its
--                                               other sessions when its
--                                               password changes. Two of the
--                                               three password paths already
--                                               did this; the switch exists so
--                                               the behaviour is visible.
--
-- The three numeric settings are stored as **strings** with type 'string',
-- not as numbers. `ghost_core::security::choice` reduces whatever is stored to
-- one of the values the screen offered and falls back to the default
-- otherwise, so a hand-edited row cannot produce a one-character password
-- floor or a session that never expires; storing them as text keeps the form
-- post, the stored row and the offered list all the same shape, which is what
-- makes that reduction a single comparison rather than three.
--
-- The boolean rows carry type 'boolean' because `SettingsCache` reads the
-- stored type to decide how to parse the value, and the Admin API serializes
-- settings from that column — getting it wrong would hand an API client a
-- string where it expects a boolean.

INSERT INTO settings (id, `group`, key, value, type, created_at, updated_at)
SELECT lower(hex(randomblob(12))), 'security', k, v, 'boolean', datetime('now'), datetime('now')
FROM (
    SELECT 'security_headers'                   AS k, 'true'  AS v UNION ALL
    SELECT 'security_hsts',                           'false'      UNION ALL
    SELECT 'security_trust_proxy',                    'true'       UNION ALL
    SELECT 'security_private_fetch',                  'true'       UNION ALL
    SELECT 'security_revoke_on_password_change',      'true'
) AS defaults
WHERE NOT EXISTS (SELECT 1 FROM settings s WHERE s.key = defaults.k);

INSERT INTO settings (id, `group`, key, value, type, created_at, updated_at)
SELECT lower(hex(randomblob(12))), 'security', k, v, 'string', datetime('now'), datetime('now')
FROM (
    SELECT 'security_admin_referrer_policy' AS k, 'same-origin' AS v UNION ALL
    SELECT 'security_session_days',               '7'                UNION ALL
    SELECT 'security_session_idle_minutes',       '0'                UNION ALL
    SELECT 'security_min_password_length',        '10'
) AS defaults
WHERE NOT EXISTS (SELECT 1 FROM settings s WHERE s.key = defaults.k);

-- ----------------------------------------------------------------------------
-- Two things the module's own report would otherwise open by reporting
-- ----------------------------------------------------------------------------
--
-- Both are consequences of the session policy this module introduces, and
-- both are cheaper to fix once here than to show an administrator as a
-- finding they have to click.
--
-- A session belonging to a suspended account. Suspending somebody has never
-- revoked their sessions — `verify_password` refuses a new sign-in, but a
-- browser that was already signed in carried on — so a database arriving at
-- this migration may well hold one, and the module's `suspended_sessions`
-- finding is Critical. The admin now revokes on suspension; this clears what
-- was already there.
DELETE FROM sessions WHERE user_id IN (SELECT id FROM users WHERE status <> 'active');

-- A session whose user no longer exists. `sessions.user_id` has no foreign
-- key, so deleting a staff member has always left the rows behind.
DELETE FROM sessions WHERE user_id NOT IN (SELECT id FROM users);
