-- The per-server quantity model flips from usage-metered (litepod pushed
-- Paddle's quantity to match the live server count) to purchase-capped
-- (the payer buys a quantity up front; litepod blocks creating a server
-- past it). Same column, new meaning: the quantity the payer has
-- purchased, not the last quantity litepod auto-synced to match usage.
ALTER TABLE billing_accounts RENAME COLUMN synced_server_quantity TO purchased_server_quantity;
