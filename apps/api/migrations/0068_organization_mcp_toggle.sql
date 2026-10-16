-- The MCP server's kill switch, per organization.
--
-- Separate from `assistant_enabled` (0026): that one decides whether Litepod
-- sends this organization's data to a model connection. MCP is the other
-- direction - the user's own agent calls in with a token - so turning one on
-- must not turn on the other. Off by default for the same reason as 0026.
ALTER TABLE organizations
    ADD COLUMN mcp_enabled INTEGER NOT NULL DEFAULT 0 CHECK (mcp_enabled IN (0,1));
