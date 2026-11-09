-- The MCP client that started a run, so its history names the agent instead of
-- crediting a person or the in-app assistant. NULL for every other run.
ALTER TABLE application_deployment_runs ADD COLUMN mcp_client TEXT;
