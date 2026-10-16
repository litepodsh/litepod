-- Historical diagnostics may contain billing PII and request credentials.
-- Reconstruct from dedicated metadata columns, even when old bodies are invalid JSON.
UPDATE billing_webhook_deliveries
SET request_headers_json = '{}',
    request_body = json_object('event_id', paddle_event_id, 'event_type', event_type),
    response_body = NULL,
    error_message = NULL;

DELETE FROM billing_webhook_deliveries
WHERE received_at < (unixepoch() - 30 * 86400) * 1000;

CREATE INDEX IF NOT EXISTS idx_billing_webhook_deliveries_received_at
ON billing_webhook_deliveries(received_at);
