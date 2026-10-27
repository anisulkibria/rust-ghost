-- Audience feedback. The "More like this / Less like this" buttons a
-- newsletter carries when its `feedback_enabled` switch is on link to
-- `#/feedback/{post}/{score}`, and Portal posts the click to
-- `/members/api/feedback`. Without somewhere to put it the link opened a modal
-- that could only fail, so the toggle in Settings → Membership promised
-- something the site could not keep.
--
-- Ghost's `members_feedback`, same columns. One row per member per post: a
-- reader changing their mind replaces their score rather than adding a second
-- one, which is what the unique index enforces.
CREATE TABLE IF NOT EXISTS members_feedback (
    id TEXT PRIMARY KEY NOT NULL,
    score INTEGER UNSIGNED NOT NULL DEFAULT 0,
    member_id TEXT(24) NOT NULL REFERENCES members(id) ON DELETE CASCADE,
    post_id TEXT(24) NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
    created_at DATETIME NOT NULL,
    updated_at DATETIME
);

CREATE UNIQUE INDEX IF NOT EXISTS members_feedback_member_post_idx
    ON members_feedback (member_id, post_id);
