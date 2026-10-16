-- Container restart policy per application, exposed on the registry-image
-- source (and applied to every container that source rolls out). Previously
-- hardcoded to `unless-stopped` in `managed_container_builder`. SQLite allows
-- a CHECK on an added column, so no table rebuild is needed here.
ALTER TABLE applications ADD COLUMN restart_policy TEXT NOT NULL DEFAULT 'unless-stopped'
    CHECK (restart_policy IN ('no', 'always', 'unless-stopped', 'on-failure'));