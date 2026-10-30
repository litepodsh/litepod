-- A git-type application can take its source tree from an uploaded ZIP instead
-- of a clone. `deploy_type` stays `git` so every build method and lifecycle
-- path keeps working; this flag only swaps the "clone" step for an unpack.
ALTER TABLE applications ADD COLUMN source_upload INTEGER NOT NULL DEFAULT 0;
