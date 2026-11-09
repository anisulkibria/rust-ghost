-- Edition-bound newsletter facts; legacy facts remain explicitly unassigned.
ALTER TABLE newsletter_clicks ADD COLUMN email_id VARCHAR(24);
ALTER TABLE click_outbox ADD COLUMN edition_id VARCHAR(24) NOT NULL DEFAULT '';
ALTER TABLE click_outbox ADD COLUMN recipient_id VARCHAR(36) NOT NULL DEFAULT '';
DROP TRIGGER click_newsletter_sent;
DROP TRIGGER click_newsletter_open;
DROP TRIGGER click_newsletter_click;
CREATE TRIGGER click_newsletter_sent AFTER INSERT ON email_recipients
WHEN (NEW.status='sent') AND (SELECT capture FROM click_connection WHERE id=1)=1
BEGIN
 INSERT OR IGNORE INTO click_outbox(id,kind,amount_minor,currency,occurred_at,available_at,edition_id,recipient_id)
 VALUES('sent:' || NEW.id,'newsletter_sent',NULL,NULL,COALESCE(NEW.created_at,datetime('now')),datetime('now'),NEW.email_id,COALESCE(NEW.member_uuid,NEW.id));
END;
CREATE TRIGGER click_newsletter_open AFTER UPDATE ON email_recipients
WHEN (NEW.opened_at IS NOT NULL AND OLD.opened_at IS NULL) AND (SELECT capture FROM click_connection WHERE id=1)=1
BEGIN
 INSERT OR IGNORE INTO click_outbox(id,kind,amount_minor,currency,occurred_at,available_at,edition_id,recipient_id)
 VALUES('open:' || NEW.id,'newsletter_open',NULL,NULL,COALESCE(NEW.opened_at,datetime('now')),datetime('now'),NEW.email_id,COALESCE(NEW.member_uuid,NEW.id));
END;
CREATE TRIGGER click_newsletter_click AFTER INSERT ON newsletter_clicks
WHEN (SELECT capture FROM click_connection WHERE id=1)=1
BEGIN
 INSERT OR IGNORE INTO click_outbox(id,kind,occurred_at,available_at,edition_id,recipient_id)
 VALUES('click:' || CASE WHEN NEW.email_id IS NULL THEN '' ELSE NEW.email_id || ':' END || NEW.link_id || ':' || NEW.member_uuid,'newsletter_click',COALESCE(NEW.created_at,datetime('now')),datetime('now'),(SELECT CASE WHEN COUNT(DISTINCT r.email_id)=1 THEN MIN(r.email_id) ELSE '' END FROM email_recipients r JOIN emails e ON e.id=r.email_id JOIN newsletter_links l ON l.post_id=e.post_id WHERE l.id=NEW.link_id AND r.member_uuid=NEW.member_uuid AND r.status='sent' AND (NEW.email_id IS NULL OR e.id=NEW.email_id)),(SELECT CASE WHEN COUNT(DISTINCT r.email_id)=1 THEN MIN(r.member_uuid) ELSE '' END FROM email_recipients r JOIN emails e ON e.id=r.email_id JOIN newsletter_links l ON l.post_id=e.post_id WHERE l.id=NEW.link_id AND r.member_uuid=NEW.member_uuid AND r.status='sent' AND (NEW.email_id IS NULL OR e.id=NEW.email_id)));
END;
