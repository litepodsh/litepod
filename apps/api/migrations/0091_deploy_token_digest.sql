-- Deploy hook tokens were stored in the clear (`applications.deploy_token`): a leaked
-- backup of litepod.db let anyone trigger deploys of every application. The token
-- is a bearer credential the server only checks, so it is stored as a SHA-256
-- digest, like session tokens (0007).
--
-- SQLite has no sha256() here, so existing tokens are hashed at boot by
-- `deployment::deploy_hook::hash_legacy_tokens`, which also clears `deploy_token`.
-- Existing deploy URLs keep working.
ALTER TABLE applications ADD COLUMN deploy_token_digest TEXT;
CREATE UNIQUE INDEX idx_applications_deploy_token_digest
    ON applications(deploy_token_digest) WHERE deploy_token_digest IS NOT NULL;
