-- ============================================================================
-- 0016_firewall.sql — the firewall's rules, and the spam switches
-- ============================================================================
--
-- The Security module's first table, and the reason it needed one: unlike the
-- posture report — which is computed from sessions, keys, staff and settings
-- that already exist — a list of barred addresses is not derivable from
-- anything. Somebody has to write it down. See
-- crates/ghost-db/src/repos/firewall.rs.
--
-- ----------------------------------------------------------------------------
-- What a rule is
-- ----------------------------------------------------------------------------
--
--   action     'block' | 'allow'. An allow is an exception somebody wrote
--              deliberately and beats a block, so a hole can be punched in a
--              range. Anything unrecognised is read as 'block' by
--              ghost_core::firewall::Action::parse — the safe reading of a
--              row nobody can parse, because a corrupted rule that refuses a
--              request is visible and fixable and one that admits a request
--              is neither.
--
--   scope      'site' | 'admin'. A site rule covers the admin too; an admin
--              rule leaves the publication readable, which is the point of
--              having two. Anything unrecognised is read as 'admin' — the
--              narrower of the two, because a rule meant for /ghost/ that
--              instead took the site down would hurt a reader who did
--              nothing.
--
--   pattern    A single address or a CIDR range, v4 or v6, stored
--              **normalized** by ghost_core::firewall::Pattern — '10.0.0.0/8'
--              and never '10.1.2.3/8'. Normalizing on the way in is what lets
--              the unique index below mean "one rule per range": without it
--              the same range spelled two ways is two rules that cannot both
--              be right.
--
--   expires_at NULL for a permanent rule. Most blocks should not be: an
--              address that attacked the site in March is usually somebody
--              else's by June, and a blocklist nobody prunes eventually
--              refuses a customer. An expired rule is filtered out on read
--              and swept at boot.
--
--   hits /     Updated when a rule actually refuses somebody, so the screen
--   last_hit_at can show which rules are doing anything. Deliberately NOT on
--              the hot path for a rule that matches nothing: only a block
--              that fired costs a write.
--
-- The unique index is on (scope, pattern) rather than on pattern alone: the
-- same address can legitimately be barred from the admin and allowed on the
-- site. It does not include `action`, so saving an allow for a range that is
-- already blocked at the same scope *replaces* it rather than producing two
-- contradictory rules — which is what somebody changing their mind means.

CREATE TABLE IF NOT EXISTS firewall_rules (
    id VARCHAR(24) PRIMARY KEY NOT NULL,
    action VARCHAR(10) NOT NULL DEFAULT 'block',
    scope VARCHAR(10) NOT NULL DEFAULT 'admin',
    pattern VARCHAR(60) NOT NULL,
    note VARCHAR(200) NOT NULL DEFAULT '',
    created_at VARCHAR(32) NOT NULL,
    created_by VARCHAR(24),
    expires_at VARCHAR(32),
    hits INTEGER NOT NULL DEFAULT 0,
    last_hit_at VARCHAR(32),
    UNIQUE KEY firewall_rules_scope_pattern (scope, pattern),
    -- Read on every request when the firewall is on, filtered by expiry.
    KEY firewall_rules_expires (expires_at)
);

-- ----------------------------------------------------------------------------
-- The switches
-- ----------------------------------------------------------------------------
--
-- **Every one of these ships off**, which is a different rule from the one
-- 0015_security.sql argued for. There, two defences shipped on because no
-- configuration existed in which they broke a working site. Each of these
-- refuses somebody, and a site that starts silently turning away signups or
-- requests because a module was installed has had a decision made for it that
-- it cannot see being made.
--
--   firewall_enabled             false  Enforce firewall_rules at all. The
--                                       admin screen turns it on when the
--                                       first rule is saved, so an empty
--                                       firewall costs nothing and the switch
--                                       is never the thing somebody forgot.
--   spam_block_disposable_email  false  Refuse member signups from the
--                                       built-in disposable-address list
--                                       (ghost_core::firewall::
--                                       DISPOSABLE_DOMAINS), on top of
--                                       whatever is in Settings → Membership
--                                       → Spam filters.
--   spam_block_user_agents       false  Refuse requests whose User-Agent
--                                       matches spam_blocked_user_agents.
--   spam_signup_same_origin      false  Require a member signup to carry this
--                                       site's origin, as the forms endpoint
--                                       already does. Off by default because
--                                       a site that embeds its signup form
--                                       elsewhere has a legitimate reason for
--                                       cross-origin signups.
--   spam_blocked_user_agents     ''     One fragment per line, matched
--                                       case-insensitively as a substring.
--                                       Fragments and not patterns: a regular
--                                       expression evaluated against a header
--                                       on every request is a denial of
--                                       service somebody can write into their
--                                       own settings by accident.

INSERT INTO settings (id, `group`, `key`, value, type, created_at, updated_at)
SELECT lower(hex(RANDOM_BYTES(12))), 'security', k, 'false', 'boolean', UTC_TIMESTAMP(), UTC_TIMESTAMP()
FROM (
    SELECT 'firewall_enabled'            AS k UNION ALL
    SELECT 'spam_block_disposable_email'      UNION ALL
    SELECT 'spam_block_user_agents'           UNION ALL
    SELECT 'spam_signup_same_origin'
) AS defaults
WHERE NOT EXISTS (SELECT 1 FROM settings s WHERE s.`key` = defaults.k);

INSERT INTO settings (id, `group`, `key`, value, type, created_at, updated_at)
SELECT lower(hex(RANDOM_BYTES(12))), 'security', 'spam_blocked_user_agents', '', 'string', UTC_TIMESTAMP(), UTC_TIMESTAMP()
WHERE NOT EXISTS (SELECT 1 FROM settings s WHERE s.`key` = 'spam_blocked_user_agents');
