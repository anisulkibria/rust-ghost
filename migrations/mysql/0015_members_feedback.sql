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
    `id` VARCHAR(191) PRIMARY KEY NOT NULL,
    `score` BIGINT UNSIGNED NOT NULL DEFAULT 0,
    `member_id` VARCHAR(191) NOT NULL ,
    `post_id` VARCHAR(191) NOT NULL ,
    `created_at` VARCHAR(32) NOT NULL,
    `updated_at` VARCHAR(32)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE UNIQUE INDEX members_feedback_member_post_idx ON members_feedback (`member_id`, `post_id`);

-- Foreign keys are added after the tables they point at.
-- SQLite resolves a reference lazily and accepts a forward one;
-- MySQL requires the referenced table to exist already.
ALTER TABLE members_feedback ADD CONSTRAINT members_feedback_member_id_fk FOREIGN KEY (`member_id`) REFERENCES members(`id`) ON DELETE CASCADE;
ALTER TABLE members_feedback ADD CONSTRAINT members_feedback_post_id_fk2 FOREIGN KEY (`post_id`) REFERENCES posts(`id`) ON DELETE CASCADE;
