-- Private connector state is deliberately outside settings and public APIs.
CREATE TABLE click_connection (
 id INTEGER PRIMARY KEY,
 config TEXT NOT NULL,
 capture INTEGER NOT NULL DEFAULT 0
);
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
);
CREATE INDEX click_outbox_due ON click_outbox(state,available_at);
CREATE TRIGGER click_member_created AFTER INSERT ON members
WHEN (1=1) AND (SELECT capture FROM click_connection WHERE id=1)=1
BEGIN
 INSERT OR IGNORE INTO click_outbox(id,kind,amount_minor,currency,occurred_at,available_at)
 VALUES('member:' || NEW.id,'member_signup',NULL,NULL,COALESCE(NEW.created_at,datetime('now')),datetime('now'));
END;
CREATE TRIGGER click_subscription_created AFTER INSERT ON subscriptions
WHEN (NEW.status IN ('active','trialing')) AND (SELECT capture FROM click_connection WHERE id=1)=1
BEGIN
 INSERT OR IGNORE INTO click_outbox(id,kind,amount_minor,currency,occurred_at,available_at)
 VALUES('subscription:' || NEW.id || ':start','subscription_started',NULL,NULL,COALESCE(NEW.created_at,datetime('now')),datetime('now'));
END;
CREATE TRIGGER click_subscription_active AFTER UPDATE ON subscriptions
WHEN (NEW.status IN ('active','trialing') AND COALESCE(OLD.status,'') NOT IN ('active','trialing')) AND (SELECT capture FROM click_connection WHERE id=1)=1
BEGIN
 INSERT OR IGNORE INTO click_outbox(id,kind,amount_minor,currency,occurred_at,available_at)
 VALUES('subscription:' || lower(hex(randomblob(16))),'subscription_started',NULL,NULL,COALESCE(NEW.updated_at,datetime('now')),datetime('now'));
END;
CREATE TRIGGER click_subscription_cancelled AFTER UPDATE ON subscriptions
WHEN (NEW.status='canceled' AND COALESCE(OLD.status,'')<>'canceled') AND (SELECT capture FROM click_connection WHERE id=1)=1
BEGIN
 INSERT OR IGNORE INTO click_outbox(id,kind,amount_minor,currency,occurred_at,available_at)
 VALUES('cancel:' || lower(hex(randomblob(16))),'subscription_cancelled',NULL,NULL,COALESCE(NEW.updated_at,datetime('now')),datetime('now'));
END;
CREATE TRIGGER click_payment AFTER INSERT ON members_payment_events
WHEN (NEW.amount>0) AND (SELECT capture FROM click_connection WHERE id=1)=1
BEGIN
 INSERT OR IGNORE INTO click_outbox(id,kind,amount_minor,currency,occurred_at,available_at)
 VALUES('payment:' || NEW.id,'payment',NEW.amount,upper(NEW.currency),COALESCE(NEW.created_at,datetime('now')),datetime('now'));
END;
CREATE TRIGGER click_refund AFTER INSERT ON members_payment_events
WHEN (NEW.amount<0) AND (SELECT capture FROM click_connection WHERE id=1)=1
BEGIN
 INSERT OR IGNORE INTO click_outbox(id,kind,amount_minor,currency,occurred_at,available_at)
 VALUES('refund:' || NEW.id,'refund',-NEW.amount,upper(NEW.currency),COALESCE(NEW.created_at,datetime('now')),datetime('now'));
END;
CREATE TRIGGER click_newsletter_sent AFTER INSERT ON email_recipients
WHEN (NEW.status='sent') AND (SELECT capture FROM click_connection WHERE id=1)=1
BEGIN
 INSERT OR IGNORE INTO click_outbox(id,kind,amount_minor,currency,occurred_at,available_at)
 VALUES('sent:' || NEW.id,'newsletter_sent',NULL,NULL,COALESCE(NEW.created_at,datetime('now')),datetime('now'));
END;
CREATE TRIGGER click_newsletter_open AFTER UPDATE ON email_recipients
WHEN (NEW.opened_at IS NOT NULL AND OLD.opened_at IS NULL) AND (SELECT capture FROM click_connection WHERE id=1)=1
BEGIN
 INSERT OR IGNORE INTO click_outbox(id,kind,amount_minor,currency,occurred_at,available_at)
 VALUES('open:' || NEW.id,'newsletter_open',NULL,NULL,COALESCE(NEW.opened_at,datetime('now')),datetime('now'));
END;
CREATE TRIGGER click_newsletter_click AFTER INSERT ON newsletter_clicks
WHEN (1=1) AND (SELECT capture FROM click_connection WHERE id=1)=1
BEGIN
 INSERT OR IGNORE INTO click_outbox(id,kind,amount_minor,currency,occurred_at,available_at)
 VALUES('click:' || NEW.link_id || ':' || NEW.member_uuid,'newsletter_click',NULL,NULL,COALESCE(NEW.created_at,datetime('now')),datetime('now'));
END;
