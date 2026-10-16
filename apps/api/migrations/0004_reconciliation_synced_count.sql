-- Reconciliation now also pull-syncs already-linked accounts straight from
-- Paddle (not just unmatched payments) — tracks how many accounts that
-- touched per run, alongside the existing processed/pending/failed/candidate
-- /unassigned counters.
ALTER TABLE billing_reconciliation_runs ADD COLUMN synced_count INTEGER NOT NULL DEFAULT 0;
