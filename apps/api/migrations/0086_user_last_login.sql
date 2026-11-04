-- When each user last signed in. `sessions` loses its rows on sign-out and expiry, and
-- security events are purged, so neither can answer this for a long-idle account.
ALTER TABLE users ADD COLUMN last_login_at TEXT;

UPDATE users SET last_login_at = (
    SELECT strftime('%Y-%m-%dT%H:%M:%SZ', MAX(created_at)) FROM (
        SELECT created_at FROM sessions WHERE user_id = users.id
        UNION ALL
        SELECT created_at FROM security_events
        WHERE user_id = users.id AND action IN ('auth.login', 'auth.login_sso', 'auth.registered')
    )
);

-- A trigger because sessions are created on three paths, one inside the invitation
-- acceptance transaction, and every sign-in creates one.
CREATE TRIGGER trg_sessions_last_login
AFTER INSERT ON sessions
BEGIN
    UPDATE users SET last_login_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = NEW.user_id;
END;
