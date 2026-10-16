-- Recovery codes were stored with `encrypt_data` (AES-256-GCM) and compared by
-- decrypting every stored code and testing it with `==`. Two problems: a code is
-- a credential that only ever needs to be *checked*, so it should be a digest
-- rather than something recoverable from the database plus the encryption key;
-- and the plain `==` compared byte by byte, leaking match length through timing.
--
-- The column becomes a SHA-256 digest, matching how invitation tokens
-- (`cqrs/commands/invitation.rs`) and email verification codes already work.
--
-- Existing rows cannot be converted: the migration cannot decrypt them, since
-- ENCRYPTION_KEY lives in the process environment and not in SQLite. They are
-- deleted instead. That is safe rather than destructive — recovery codes are
-- only reachable for accounts with TOTP enabled, and those accounts keep working
-- through their authenticator. Anyone whose codes are dropped regenerates them
-- from the account page, which is also what they would have to do after using
-- one.

BEGIN;

DELETE FROM recovery_codes;

ALTER TABLE recovery_codes RENAME COLUMN encrypted_code TO code_digest;

COMMIT;
