-- Actual provider submissions, shared by transactional and newsletter mail.
CREATE TABLE mail_send_events (
    id TEXT PRIMARY KEY,
    period_start TEXT NOT NULL,
    kind TEXT NOT NULL,
    provider TEXT NOT NULL,
    recipient_count BIGINT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX mail_send_events_created ON mail_send_events(created_at);
CREATE INDEX mail_send_events_period ON mail_send_events(period_start, status);
CREATE TABLE mail_send_periods (
    period_start TEXT PRIMARY KEY,
    sent BIGINT NOT NULL DEFAULT 0,
    reserved BIGINT NOT NULL DEFAULT 0
);
-- Preserve known newsletter submissions; historical transactional mail was not logged.
INSERT INTO mail_send_events (id,period_start,kind,provider,recipient_count,status,created_at)
SELECT 'legacy-' || id, '', 'newsletter', 'legacy', delivered_count, 'sent', strftime('%Y-%m-%dT%H:%M:%SZ', created_at)
FROM emails WHERE delivered_count > 0 AND created_at IS NOT NULL;
