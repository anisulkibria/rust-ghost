-- Actual provider submissions, shared by transactional and newsletter mail.
CREATE TABLE mail_send_events (
    id VARCHAR(64) PRIMARY KEY,
    period_start VARCHAR(40) NOT NULL,
    kind VARCHAR(32) NOT NULL,
    provider VARCHAR(32) NOT NULL,
    recipient_count BIGINT NOT NULL,
    status VARCHAR(32) NOT NULL,
    created_at VARCHAR(40) NOT NULL
);
CREATE INDEX mail_send_events_created ON mail_send_events(created_at);
CREATE INDEX mail_send_events_period ON mail_send_events(period_start, status);
CREATE TABLE mail_send_periods (
    period_start VARCHAR(40) PRIMARY KEY,
    sent BIGINT NOT NULL DEFAULT 0,
    reserved BIGINT NOT NULL DEFAULT 0
);
-- Preserve known newsletter submissions; historical transactional mail was not logged.
INSERT INTO mail_send_events (id,period_start,kind,provider,recipient_count,status,created_at)
SELECT CONCAT('legacy-', id), '', 'newsletter', 'legacy', delivered_count, 'sent', DATE_FORMAT(created_at, '%Y-%m-%dT%H:%i:%sZ')
FROM emails WHERE delivered_count > 0 AND created_at IS NOT NULL;
