-- Run history is listed `ORDER BY started_at DESC, id DESC`. With `id` in the index
-- SQLite reads rows in that order instead of sorting them in a temporary B-tree.
DROP INDEX IF EXISTS idx_deployment_runs_application_started;
CREATE INDEX idx_deployment_runs_application_started
    ON application_deployment_runs(application_id, started_at DESC, id DESC);

-- The retention purge compares `created_at` per organization; without this it scans
-- the whole table every day.
CREATE INDEX idx_audit_logs_organization_created_at ON audit_logs(organization_id, created_at);
