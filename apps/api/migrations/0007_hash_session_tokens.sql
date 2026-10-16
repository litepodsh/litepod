-- Session tokens were stored in the clear: `sessions.token` held the exact
-- string sitting in the user's cookie. Anyone who could read the table — a
-- backup, a replica, a SQL-injection foothold, an operator with a shell — could
-- take any live session by copying a value out of it. A session token is a
-- bearer credential; the server only ever needs to *check* one, never read it
-- back, so it belongs stored as a digest, exactly like invitation tokens
-- (`cqrs/commands/invitation.rs`), email verification codes and — since
-- migration 0006 — recovery codes.
--
-- The column becomes a SHA-256 digest of the token.
--
-- Existing rows are deleted rather than converted. They could in principle be
-- hashed in place, but SQLite has no sha256() here, and a session that survives
-- a credential-storage change is exactly the thing this migration exists to
-- make impossible to steal. Everyone signs in again once. That is also why the
-- accompanying code change lands together with `Secure` on the cookie: both
-- take effect at the same re-login.

BEGIN;

DELETE FROM sessions;

DROP INDEX IF EXISTS idx_sessions_token;

ALTER TABLE sessions RENAME COLUMN token TO token_digest;

CREATE INDEX idx_sessions_token_digest ON sessions(token_digest);

COMMIT;
