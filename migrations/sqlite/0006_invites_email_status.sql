-- Staff invites need an email and a status.
--
-- The initial schema carried only `role_id`, `token` and `expires`, which is
-- enough to *redeem* an invite but not to show one: Settings → Staff lists
-- pending invitations by address, and resending one has to know where to send
-- it. Ghost's own `invites` table has both columns (status is
-- 'pending' | 'sent'), so this brings the table in line.
--
-- SQLite's ALTER TABLE ADD COLUMN cannot add a UNIQUE column, so uniqueness
-- on email is enforced by a separate index — which is what we want anyway:
-- one outstanding invite per address.
ALTER TABLE invites ADD COLUMN email TEXT(191) NOT NULL DEFAULT '';
ALTER TABLE invites ADD COLUMN status TEXT(50) NOT NULL DEFAULT 'pending';

CREATE UNIQUE INDEX IF NOT EXISTS invites_email_unique ON invites (email);
