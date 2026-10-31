-- Lets an organization require two-factor authentication from every member.
--
-- Separate from `sso_enforced` (0053): that one decides how a member signs in,
-- this one decides whether a member who has signed in may act inside the
-- organization without a second factor. Off by default so existing
-- organizations behave exactly as before until an admin turns it on.
ALTER TABLE organizations
    ADD COLUMN require_two_factor INTEGER NOT NULL DEFAULT 0 CHECK (require_two_factor IN (0,1));
