-- The Forms module.
--
-- Consolidated from earlier migrations. Each section below is one of them (or the
-- named part of one), unchanged and in the order it was first applied.

-- ============================================================================
-- forms  (was 0041_forms.sql)
-- ============================================================================

-- Forms: reusable contact-style forms, embedded in posts and pages by the
-- editor's form card, and the responses visitors send through them.
CREATE TABLE forms (
 id VARCHAR(24) PRIMARY KEY,
 name VARCHAR(191) NOT NULL,
 -- 'active' accepts responses; 'closed' renders a notice and refuses them.
 status VARCHAR(20) NOT NULL DEFAULT 'active',
 -- The field definitions, as JSON. See ghost_core::forms::Field.
 fields MEDIUMTEXT NOT NULL,
 submit_label VARCHAR(100) NOT NULL DEFAULT 'Send',
 success_message VARCHAR(500) NOT NULL DEFAULT '',
 -- Comma-separated addresses told about each response. Empty falls back to
 -- the site-wide list in forms_config.
 notify_emails VARCHAR(1000) NOT NULL DEFAULT '',
 created_at VARCHAR(32) NOT NULL,
 updated_at VARCHAR(32) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE form_submissions (
 id VARCHAR(24) PRIMARY KEY,
 form_id VARCHAR(24) NOT NULL,
 -- The form's name when the response arrived, so a rename does not relabel
 -- history.
 form_name VARCHAR(191) NOT NULL,
 -- The answers, as JSON, each carrying the label it was asked under.
 data MEDIUMTEXT NOT NULL,
 -- 'new', 'read' or 'spam'.
 status VARCHAR(20) NOT NULL DEFAULT 'new',
 -- The single-use token this response spent. UNIQUE is what makes a replayed
 -- request fail instead of filing the same response twice.
 nonce VARCHAR(32) NOT NULL UNIQUE,
 ip VARCHAR(64),
 user_agent VARCHAR(300),
 page_path VARCHAR(500),
 member_id VARCHAR(24),
 created_at VARCHAR(32) NOT NULL,
 CONSTRAINT form_submissions_form_fk FOREIGN KEY (form_id) REFERENCES forms(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX form_submissions_form ON form_submissions(form_id, created_at);
CREATE INDEX form_submissions_status ON form_submissions(status, created_at);
CREATE INDEX form_submissions_created ON form_submissions(created_at);

-- Site-wide forms settings: one row. Kept out of `settings` so nothing here is
-- ever serialised by the settings APIs — `secret` signs the submission tokens.
CREATE TABLE forms_config (
 id INTEGER PRIMARY KEY,
 notify_enabled INTEGER NOT NULL DEFAULT 1,
 notify_emails VARCHAR(1000) NOT NULL DEFAULT '',
 store_ip INTEGER NOT NULL DEFAULT 1,
 retention_days INTEGER NOT NULL DEFAULT 0,
 secret VARCHAR(64) NOT NULL DEFAULT ''
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
INSERT INTO forms_config(id) VALUES(1);

-- ============================================================================
-- form_success_action  (was 0042_form_success_action.sql)
-- ============================================================================

-- What a form does once a response is accepted: show its message
-- ('message'), show the message with a link to a URL or file ('link'), or
-- send the visitor on to a URL ('redirect').
ALTER TABLE forms ADD COLUMN success_action VARCHAR(20) NOT NULL DEFAULT 'message';
ALTER TABLE forms ADD COLUMN success_url VARCHAR(2000) NOT NULL DEFAULT '';
ALTER TABLE forms ADD COLUMN success_link_label VARCHAR(100) NOT NULL DEFAULT '';
