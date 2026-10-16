-- Personal access tokens: self-service, per-user (like a GitHub PAT), not
-- issued by an org admin. A token acts with whatever role its owner already
-- has in an organization (see org_scope.rs), capped by its own scopes — if
-- the owner loses membership, the token stops working with no cleanup job
-- needed, since the role lookup is the same one session auth already uses.
CREATE TABLE personal_access_tokens (
    id            TEXT PRIMARY KEY,
    user_id       TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    -- First chars of the raw token, kept in the clear only to let the owner
    -- recognize which token is which in the list — never enough to guess the
    -- rest, which lives only as token_digest.
    token_prefix  TEXT NOT NULL,
    token_digest  TEXT NOT NULL UNIQUE,
    -- JSON array, subset of ["read","write","deploy"].
    scopes        TEXT NOT NULL,
    -- NULL = never expires; capped at 365 days from creation at the
    -- application layer (see dtos::personal_access_token).
    expires_at    TEXT,
    last_used_at  TEXT,
    revoked_at    TEXT,
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%S', 'now'))
);
CREATE INDEX idx_personal_access_tokens_user_id ON personal_access_tokens(user_id);
