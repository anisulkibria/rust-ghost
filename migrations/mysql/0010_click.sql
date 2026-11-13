-- The Click connector: its connection row, outbox and the triggers that feed it.
--
-- Consolidated from earlier migrations. Each section below is one of them (or the
-- named part of one), unchanged and in the order it was first applied.

-- ============================================================================
-- click_connector  (was 0038_click_connector.sql)
-- ============================================================================

-- Private connector state is deliberately outside settings and public APIs.
CREATE TABLE click_connection (
 id INTEGER PRIMARY KEY,
 config LONGTEXT NOT NULL,
 capture INTEGER NOT NULL DEFAULT 0
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
INSERT INTO click_connection(id,config) VALUES(1,'null');
CREATE TABLE click_outbox (
 id VARCHAR(191) PRIMARY KEY,
 kind VARCHAR(50) NOT NULL,
 amount_minor BIGINT,
 currency VARCHAR(3),
 occurred_at VARCHAR(40) NOT NULL,
 state VARCHAR(20) NOT NULL DEFAULT 'pending',
 attempts INTEGER NOT NULL DEFAULT 0,
 available_at VARCHAR(40) NOT NULL,
 last_error VARCHAR(100)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
CREATE INDEX click_outbox_due ON click_outbox(state,available_at);
CREATE TRIGGER click_member_created AFTER INSERT ON members FOR EACH ROW
BEGIN
 IF (1=1) AND (SELECT capture FROM click_connection WHERE id=1)=1 THEN
 INSERT IGNORE INTO click_outbox(id,kind,amount_minor,currency,occurred_at,available_at)
 VALUES(CONCAT('member:',NEW.id),'member_signup',NULL,NULL,COALESCE(NEW.created_at,DATE_FORMAT(UTC_TIMESTAMP(),'%Y-%m-%d %H:%i:%s')),DATE_FORMAT(UTC_TIMESTAMP(),'%Y-%m-%d %H:%i:%s'));
 END IF;
END;
CREATE TRIGGER click_subscription_created AFTER INSERT ON subscriptions FOR EACH ROW
BEGIN
 IF (NEW.status IN ('active','trialing')) AND (SELECT capture FROM click_connection WHERE id=1)=1 THEN
 INSERT IGNORE INTO click_outbox(id,kind,amount_minor,currency,occurred_at,available_at)
 VALUES(CONCAT('subscription:',NEW.id,':start'),'subscription_started',NULL,NULL,COALESCE(NEW.created_at,DATE_FORMAT(UTC_TIMESTAMP(),'%Y-%m-%d %H:%i:%s')),DATE_FORMAT(UTC_TIMESTAMP(),'%Y-%m-%d %H:%i:%s'));
 END IF;
END;
CREATE TRIGGER click_subscription_active AFTER UPDATE ON subscriptions FOR EACH ROW
BEGIN
 IF (NEW.status IN ('active','trialing') AND COALESCE(OLD.status,'') NOT IN ('active','trialing')) AND (SELECT capture FROM click_connection WHERE id=1)=1 THEN
 INSERT IGNORE INTO click_outbox(id,kind,amount_minor,currency,occurred_at,available_at)
 VALUES(CONCAT('subscription:',REPLACE(UUID(),'-','')),'subscription_started',NULL,NULL,COALESCE(NEW.updated_at,DATE_FORMAT(UTC_TIMESTAMP(),'%Y-%m-%d %H:%i:%s')),DATE_FORMAT(UTC_TIMESTAMP(),'%Y-%m-%d %H:%i:%s'));
 END IF;
END;
CREATE TRIGGER click_subscription_cancelled AFTER UPDATE ON subscriptions FOR EACH ROW
BEGIN
 IF (NEW.status='canceled' AND COALESCE(OLD.status,'')<>'canceled') AND (SELECT capture FROM click_connection WHERE id=1)=1 THEN
 INSERT IGNORE INTO click_outbox(id,kind,amount_minor,currency,occurred_at,available_at)
 VALUES(CONCAT('cancel:',REPLACE(UUID(),'-','')),'subscription_cancelled',NULL,NULL,COALESCE(NEW.updated_at,DATE_FORMAT(UTC_TIMESTAMP(),'%Y-%m-%d %H:%i:%s')),DATE_FORMAT(UTC_TIMESTAMP(),'%Y-%m-%d %H:%i:%s'));
 END IF;
END;
CREATE TRIGGER click_payment AFTER INSERT ON members_payment_events FOR EACH ROW
BEGIN
 IF (NEW.amount>0) AND (SELECT capture FROM click_connection WHERE id=1)=1 THEN
 INSERT IGNORE INTO click_outbox(id,kind,amount_minor,currency,occurred_at,available_at)
 VALUES(CONCAT('payment:',NEW.id),'payment',NEW.amount,upper(NEW.currency),COALESCE(NEW.created_at,DATE_FORMAT(UTC_TIMESTAMP(),'%Y-%m-%d %H:%i:%s')),DATE_FORMAT(UTC_TIMESTAMP(),'%Y-%m-%d %H:%i:%s'));
 END IF;
END;
CREATE TRIGGER click_refund AFTER INSERT ON members_payment_events FOR EACH ROW
BEGIN
 IF (NEW.amount<0) AND (SELECT capture FROM click_connection WHERE id=1)=1 THEN
 INSERT IGNORE INTO click_outbox(id,kind,amount_minor,currency,occurred_at,available_at)
 VALUES(CONCAT('refund:',NEW.id),'refund',-NEW.amount,upper(NEW.currency),COALESCE(NEW.created_at,DATE_FORMAT(UTC_TIMESTAMP(),'%Y-%m-%d %H:%i:%s')),DATE_FORMAT(UTC_TIMESTAMP(),'%Y-%m-%d %H:%i:%s'));
 END IF;
END;
CREATE TRIGGER click_newsletter_sent AFTER INSERT ON email_recipients FOR EACH ROW
BEGIN
 IF (NEW.status='sent') AND (SELECT capture FROM click_connection WHERE id=1)=1 THEN
 INSERT IGNORE INTO click_outbox(id,kind,amount_minor,currency,occurred_at,available_at)
 VALUES(CONCAT('sent:',NEW.id),'newsletter_sent',NULL,NULL,COALESCE(NEW.created_at,DATE_FORMAT(UTC_TIMESTAMP(),'%Y-%m-%d %H:%i:%s')),DATE_FORMAT(UTC_TIMESTAMP(),'%Y-%m-%d %H:%i:%s'));
 END IF;
END;
CREATE TRIGGER click_newsletter_open AFTER UPDATE ON email_recipients FOR EACH ROW
BEGIN
 IF (NEW.opened_at IS NOT NULL AND OLD.opened_at IS NULL) AND (SELECT capture FROM click_connection WHERE id=1)=1 THEN
 INSERT IGNORE INTO click_outbox(id,kind,amount_minor,currency,occurred_at,available_at)
 VALUES(CONCAT('open:',NEW.id),'newsletter_open',NULL,NULL,COALESCE(NEW.opened_at,DATE_FORMAT(UTC_TIMESTAMP(),'%Y-%m-%d %H:%i:%s')),DATE_FORMAT(UTC_TIMESTAMP(),'%Y-%m-%d %H:%i:%s'));
 END IF;
END;
CREATE TRIGGER click_newsletter_click AFTER INSERT ON newsletter_clicks FOR EACH ROW
BEGIN
 IF (1=1) AND (SELECT capture FROM click_connection WHERE id=1)=1 THEN
 INSERT IGNORE INTO click_outbox(id,kind,amount_minor,currency,occurred_at,available_at)
 VALUES(CONCAT('click:',NEW.link_id,':',NEW.member_uuid),'newsletter_click',NULL,NULL,COALESCE(NEW.created_at,DATE_FORMAT(UTC_TIMESTAMP(),'%Y-%m-%d %H:%i:%s')),DATE_FORMAT(UTC_TIMESTAMP(),'%Y-%m-%d %H:%i:%s'));
 END IF;
END;

-- ============================================================================
-- click_target  (was 0039_click_target.sql)
-- ============================================================================

-- A restored managed site may have a fresh analytics mapping. Never replay
-- pending records from its old website into the new mapping.
ALTER TABLE click_connection ADD COLUMN target_website VARCHAR(36) NOT NULL DEFAULT '';

-- ============================================================================
-- click_newsletters  (was 0040_click_newsletters.sql)
-- ============================================================================

-- Edition-bound newsletter facts; legacy facts remain explicitly unassigned.
ALTER TABLE newsletter_clicks ADD COLUMN email_id VARCHAR(24);
ALTER TABLE click_outbox ADD COLUMN edition_id VARCHAR(24) NOT NULL DEFAULT '';
ALTER TABLE click_outbox ADD COLUMN recipient_id VARCHAR(36) NOT NULL DEFAULT '';
DROP TRIGGER click_newsletter_sent;
DROP TRIGGER click_newsletter_open;
DROP TRIGGER click_newsletter_click;
CREATE TRIGGER click_newsletter_sent AFTER INSERT ON email_recipients FOR EACH ROW
BEGIN
 IF (NEW.status='sent') AND (SELECT capture FROM click_connection WHERE id=1)=1 THEN
 INSERT IGNORE INTO click_outbox(id,kind,amount_minor,currency,occurred_at,available_at,edition_id,recipient_id)
 VALUES(CONCAT('sent:',NEW.id),'newsletter_sent',NULL,NULL,COALESCE(NEW.created_at,DATE_FORMAT(UTC_TIMESTAMP(),'%Y-%m-%d %H:%i:%s')),DATE_FORMAT(UTC_TIMESTAMP(),'%Y-%m-%d %H:%i:%s'),NEW.email_id,COALESCE(NEW.member_uuid,NEW.id));
 END IF;
END;
CREATE TRIGGER click_newsletter_open AFTER UPDATE ON email_recipients FOR EACH ROW
BEGIN
 IF (NEW.opened_at IS NOT NULL AND OLD.opened_at IS NULL) AND (SELECT capture FROM click_connection WHERE id=1)=1 THEN
 INSERT IGNORE INTO click_outbox(id,kind,amount_minor,currency,occurred_at,available_at,edition_id,recipient_id)
 VALUES(CONCAT('open:',NEW.id),'newsletter_open',NULL,NULL,COALESCE(NEW.opened_at,DATE_FORMAT(UTC_TIMESTAMP(),'%Y-%m-%d %H:%i:%s')),DATE_FORMAT(UTC_TIMESTAMP(),'%Y-%m-%d %H:%i:%s'),NEW.email_id,COALESCE(NEW.member_uuid,NEW.id));
 END IF;
END;
CREATE TRIGGER click_newsletter_click AFTER INSERT ON newsletter_clicks FOR EACH ROW
BEGIN
 IF (SELECT capture FROM click_connection WHERE id=1)=1 THEN
 INSERT IGNORE INTO click_outbox(id,kind,occurred_at,available_at,edition_id,recipient_id)
 VALUES(CONCAT('click:',CASE WHEN NEW.email_id IS NULL THEN '' ELSE CONCAT(NEW.email_id,':') END,NEW.link_id,':',NEW.member_uuid),'newsletter_click',COALESCE(NEW.created_at,DATE_FORMAT(UTC_TIMESTAMP(),'%Y-%m-%d %H:%i:%s')),DATE_FORMAT(UTC_TIMESTAMP(),'%Y-%m-%d %H:%i:%s'),(SELECT CASE WHEN COUNT(DISTINCT r.email_id)=1 THEN MIN(r.email_id) ELSE '' END FROM email_recipients r JOIN emails e ON e.id=r.email_id JOIN newsletter_links l ON l.post_id=e.post_id WHERE l.id=NEW.link_id AND r.member_uuid=NEW.member_uuid AND r.status='sent' AND (NEW.email_id IS NULL OR e.id=NEW.email_id)),(SELECT CASE WHEN COUNT(DISTINCT r.email_id)=1 THEN MIN(r.member_uuid) ELSE '' END FROM email_recipients r JOIN emails e ON e.id=r.email_id JOIN newsletter_links l ON l.post_id=e.post_id WHERE l.id=NEW.link_id AND r.member_uuid=NEW.member_uuid AND r.status='sent' AND (NEW.email_id IS NULL OR e.id=NEW.email_id)));
 END IF;
END;
