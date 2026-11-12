-- Forms: reusable contact-style forms, embedded in posts and pages by the
-- editor's form card, and the responses visitors send through them.
CREATE TABLE forms (
 id VARCHAR(24) PRIMARY KEY,
 name VARCHAR(191) NOT NULL,
 -- 'active' accepts responses; 'closed' renders a notice and refuses them.
 status VARCHAR(20) NOT NULL DEFAULT 'active',
 -- The field definitions, as JSON. See ghost_core::forms::Field.
 fields TEXT NOT NULL,
 submit_label VARCHAR(100) NOT NULL DEFAULT 'Send',
 success_message VARCHAR(500) NOT NULL DEFAULT '',
 -- Comma-separated addresses told about each response. Empty falls back to
 -- the site-wide list in forms_config.
 notify_emails VARCHAR(1000) NOT NULL DEFAULT '',
 created_at TEXT NOT NULL,
 updated_at TEXT NOT NULL
);

CREATE TABLE form_submissions (
 id VARCHAR(24) PRIMARY KEY,
 form_id VARCHAR(24) NOT NULL REFERENCES forms(id) ON DELETE CASCADE,
 -- The form's name when the response arrived, so a rename does not relabel
 -- history.
 form_name VARCHAR(191) NOT NULL,
 -- The answers, as JSON, each carrying the label it was asked under.
 data TEXT NOT NULL,
 -- 'new', 'read' or 'spam'.
 status VARCHAR(20) NOT NULL DEFAULT 'new',
 -- The single-use token this response spent. UNIQUE is what makes a replayed
 -- request fail instead of filing the same response twice.
 nonce VARCHAR(32) NOT NULL UNIQUE,
 ip VARCHAR(64),
 user_agent VARCHAR(300),
 page_path VARCHAR(500),
 member_id VARCHAR(24),
 created_at TEXT NOT NULL
);
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
);
INSERT INTO forms_config(id) VALUES(1);
