-- The assistant's kill switch, per organization.
--
-- Off by default, deliberately: an organization that has never been asked
-- should not start sending its logs and compose files to a model because a key
-- appeared in the environment. Turning it on is a decision someone makes.
ALTER TABLE organizations
    ADD COLUMN assistant_enabled INTEGER NOT NULL DEFAULT 0 CHECK (assistant_enabled IN (0,1));
