-- Billing moves from Paddle to Polar.sh. Paddle never reached production
-- (domain approval stayed pending), so every billing row is sandbox test
-- data and is dropped rather than mapped. Paddle-named columns are renamed
-- to their Polar equivalents; SQLite keeps PKs and UNIQUE constraints
-- across RENAME COLUMN.
DELETE FROM billing_quantity_sync_attempts;
DELETE FROM billing_external_records;
DELETE FROM billing_reconciliation_runs;
DELETE FROM billing_webhook_deliveries;
DELETE FROM billing_invoices;
DELETE FROM billing_events;
DELETE FROM billing_checkout_bindings;
DELETE FROM billing_accounts;

ALTER TABLE billing_accounts RENAME COLUMN paddle_customer_id TO polar_customer_id;
ALTER TABLE billing_accounts RENAME COLUMN paddle_subscription_id TO polar_subscription_id;
ALTER TABLE billing_accounts RENAME COLUMN paddle_product_id TO polar_product_id;
ALTER TABLE billing_accounts RENAME COLUMN paddle_price_id TO polar_price_id;

-- A Polar checkout session replaces a Paddle transaction as the binding key.
ALTER TABLE billing_checkout_bindings RENAME COLUMN paddle_transaction_id TO polar_checkout_id;
ALTER TABLE billing_checkout_bindings RENAME COLUMN paddle_price_id TO polar_price_id;

-- Polar has no event id in the body; this holds the `webhook-id` header.
ALTER TABLE billing_events RENAME COLUMN paddle_event_id TO polar_event_id;

-- A Polar order replaces a Paddle transaction as the invoiced unit.
ALTER TABLE billing_invoices RENAME COLUMN paddle_transaction_id TO polar_order_id;
ALTER TABLE billing_invoices RENAME COLUMN paddle_subscription_id TO polar_subscription_id;

ALTER TABLE billing_webhook_deliveries RENAME COLUMN paddle_event_id TO polar_event_id;

ALTER TABLE billing_external_records RENAME COLUMN paddle_subscription_id TO polar_subscription_id;
ALTER TABLE billing_external_records RENAME COLUMN paddle_customer_id TO polar_customer_id;
ALTER TABLE billing_external_records RENAME COLUMN paddle_product_id TO polar_product_id;
ALTER TABLE billing_external_records RENAME COLUMN paddle_price_id TO polar_price_id;

ALTER TABLE billing_quantity_sync_attempts RENAME COLUMN paddle_subscription_id TO polar_subscription_id;
