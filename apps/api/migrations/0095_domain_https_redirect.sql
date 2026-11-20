-- Whether plain HTTP requests to an HTTPS domain are redirected to HTTPS (308).
-- On by default; a domain behind a proxy that talks HTTP to the origin (Cloudflare
-- "Flexible") turns it off so the redirect does not loop. Path domains follow
-- their host's root domain.
ALTER TABLE application_domains ADD COLUMN https_redirect INTEGER NOT NULL DEFAULT 1;
