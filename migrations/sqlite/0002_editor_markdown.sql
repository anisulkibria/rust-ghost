-- Markdown source for posts authored in the built-in editor (round-trip).
-- Ghost itself stores editor state in mobiledoc/lexical; ghost-rust's admin
-- editor is markdown-first so it keeps the source here. Extra columns don't
-- affect INSERT INTO ... SELECT imports from a real Ghost database.
ALTER TABLE posts ADD COLUMN markdown TEXT;
