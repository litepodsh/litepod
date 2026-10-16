-- Proof that a cloud account accepted the Terms of Service, Acceptable Use
-- Policy and Privacy Policy, and which version of them. Append-only: a new
-- version accepted later is a new row, so the history of what each account
-- agreed to, and when, is never rewritten.

CREATE TABLE legal_acceptances (
  id          TEXT PRIMARY KEY,
  user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  version     TEXT NOT NULL,
  accepted_at INTEGER NOT NULL,
  ip          TEXT,
  user_agent  TEXT
);

CREATE INDEX idx_legal_acceptances_user ON legal_acceptances(user_id, accepted_at DESC);
