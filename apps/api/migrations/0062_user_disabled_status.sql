-- A SysAdmin can disable a user. Distinct from 'Inactive' (self-deactivated or
-- SCIM), which re-signup and SCIM may undo; only a SysAdmin lifts 'Disabled'.
INSERT INTO status (status_id, name, description, status_type_id) VALUES
    (4, 'Disabled', 'User disabled by a SysAdmin', 1);
