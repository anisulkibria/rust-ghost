-- A restored managed site may have a fresh analytics mapping. Never replay
-- pending records from its old website into the new mapping.
ALTER TABLE click_connection ADD COLUMN target_website VARCHAR(36) NOT NULL DEFAULT '';
