-- Per-org retention window for the container metrics history kept in DuckDB
-- (src/metrics_history). NULL means "use the install default"
-- (LITEPOD_METRICS_RETENTION_DAYS, 7 when unset), so the env var can move the
-- default without rewriting every row. Capped at 365 days like 0014; the API
-- re-checks it so a bad value 400s instead of hitting the CHECK as a 500.
ALTER TABLE organizations ADD COLUMN metrics_retention_days INTEGER CHECK (metrics_retention_days IS NULL OR metrics_retention_days BETWEEN 1 AND 365);
