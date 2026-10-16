-- Polar keeps a subscription `active`/`trialing` after a cancel request until
-- the period ends. This flag lets the UI say "cancels on …" and lets the API
-- refuse slot changes Polar would reject (AlreadyCanceledSubscription).
ALTER TABLE billing_accounts ADD COLUMN cancel_at_period_end INTEGER NOT NULL DEFAULT 0;
