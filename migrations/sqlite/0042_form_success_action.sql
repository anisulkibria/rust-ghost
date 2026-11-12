-- What a form does once a response is accepted: show its message
-- ('message'), show the message with a link to a URL or file ('link'), or
-- send the visitor on to a URL ('redirect').
ALTER TABLE forms ADD COLUMN success_action VARCHAR(20) NOT NULL DEFAULT 'message';
ALTER TABLE forms ADD COLUMN success_url VARCHAR(2000) NOT NULL DEFAULT '';
ALTER TABLE forms ADD COLUMN success_link_label VARCHAR(100) NOT NULL DEFAULT '';
