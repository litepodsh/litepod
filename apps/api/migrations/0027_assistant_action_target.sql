-- An action row records the whole target, not just the application.
--
-- Renaming an application goes through the project command, which takes the
-- full organization/project/environment/application path. Those ids were known
-- when the action was stored; deriving them again at confirm time - or worse,
-- parsing them back out of `route` - would be two ways to get the same answer.
--
-- Existing rows predate any tool that needs them, so an empty default is
-- correct rather than a placeholder to clean up later.
ALTER TABLE assistant_actions ADD COLUMN project_id TEXT NOT NULL DEFAULT '';
ALTER TABLE assistant_actions ADD COLUMN environment_id TEXT NOT NULL DEFAULT '';
