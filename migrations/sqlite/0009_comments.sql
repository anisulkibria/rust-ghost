-- Comments: the indexes the thread queries lean on.
--
-- The `comments` / `comment_likes` / `comment_reports` tables were created in
-- 0001 as schema-only placeholders, with just the `(post_id, status)` index
-- Ghost has. Reading a thread also walks the table by `parent_id` (replies),
-- by `in_reply_to_id` (the direct-reply count) and by `member_id` (the
-- moderation screen's per-member view), and each of those is a full scan
-- without an index — on a busy post that is the whole comments table per
-- rendered page.
CREATE INDEX IF NOT EXISTS comments_parent_id_idx ON comments (parent_id, status);
CREATE INDEX IF NOT EXISTS comments_in_reply_to_id_idx ON comments (in_reply_to_id, status);
CREATE INDEX IF NOT EXISTS comments_member_id_idx ON comments (member_id, status);
CREATE INDEX IF NOT EXISTS comments_created_at_idx ON comments (created_at, id);

-- The unique indexes from 0001 cover `(comment_id, member_id)` lookups, but
-- not the `comment_id`-only counts that every listed comment needs.
CREATE INDEX IF NOT EXISTS comment_likes_comment_id_idx ON comment_likes (comment_id);
CREATE INDEX IF NOT EXISTS comment_reports_comment_id_idx ON comment_reports (comment_id);
