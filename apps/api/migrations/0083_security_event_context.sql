-- Where a security event came from, so a SysAdmin can tell a login from the
-- usual office apart from one on another continent.
--
-- `source_ip` / `user_agent` come from the request (`RequestContext`). The geo
-- columns are filled afterwards, on cloud only, from ip.guide for auth events;
-- a row with an IP but no country was private, unresolvable or self-hosted.
--
-- `flag` marks rows the detector found unusual (`new_country`,
-- `impossible_travel`, `distributed_failures`); NULL means nothing stood out.
ALTER TABLE security_events ADD COLUMN source_ip TEXT;
ALTER TABLE security_events ADD COLUMN user_agent TEXT;
ALTER TABLE security_events ADD COLUMN country TEXT;
ALTER TABLE security_events ADD COLUMN city TEXT;
ALTER TABLE security_events ADD COLUMN latitude REAL;
ALTER TABLE security_events ADD COLUMN longitude REAL;
ALTER TABLE security_events ADD COLUMN asn TEXT;
ALTER TABLE security_events ADD COLUMN flag TEXT;

CREATE INDEX idx_security_events_ip ON security_events(source_ip, id DESC);
CREATE INDEX idx_security_events_action ON security_events(action, id DESC);
CREATE INDEX idx_security_events_flag ON security_events(id DESC) WHERE flag IS NOT NULL;
