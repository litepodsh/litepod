-- Named S3 backup targets belong to one organization.
--
-- Until now every named target was global: any member of any organization could
-- list it (endpoint, bucket, access key id) and point a database backup policy
-- at it. On cloud that let one customer see, and write into, another's bucket.
--
-- `organization_id` stays NULL only on the default target (`is_default = 1`),
-- which is the operator's system-backup destination and is never offered to an
-- organization. Existing named targets are assigned below so no working policy
-- loses its destination:
--   1. used by policies in exactly one organization -> that organization;
--   2. used by policies in several organizations    -> one copy per organization,
--      and each organization's policies are repointed to its own copy (the
--      original row stays as the record older backups were uploaded to);
--   3. used by no policy                            -> the root organization.

ALTER TABLE backup_s3_targets ADD COLUMN organization_id TEXT REFERENCES organizations(id) ON DELETE CASCADE;

-- Names become unique per organization (index at the end). The global index
-- goes first: the per-organization copies below reuse their original's name.
DROP INDEX idx_backup_s3_targets_name;

-- Which organization each policy's application lives in.
CREATE TEMP TABLE policy_target_orgs AS
SELECT DISTINCT p.s3_target_id AS target_id, pr.organization_id AS organization_id
FROM database_backup_policies p
JOIN applications a ON a.id = p.application_id
JOIN environments e ON e.id = a.environment_id
JOIN projects pr ON pr.id = e.project_id
JOIN backup_s3_targets t ON t.id = p.s3_target_id AND t.is_default = 0
WHERE p.s3_target_id IS NOT NULL;

-- 2. Targets shared by several organizations: one copy per organization.
INSERT INTO backup_s3_targets
  (id, name, endpoint_url, bucket, region, access_key_id, wrapped_dek, dek_nonce,
   ciphertext, nonce, created_at, updated_at, created_by, is_default,
   schedule_enabled, schedule_interval, schedule_time, organization_id)
SELECT t.id || ':' || o.organization_id, t.name, t.endpoint_url, t.bucket, t.region,
       t.access_key_id, t.wrapped_dek, t.dek_nonce, t.ciphertext, t.nonce,
       t.created_at, t.updated_at, t.created_by, 0, 0, NULL, NULL, o.organization_id
FROM backup_s3_targets t
JOIN policy_target_orgs o ON o.target_id = t.id
WHERE (SELECT COUNT(*) FROM policy_target_orgs c WHERE c.target_id = t.id) > 1;

UPDATE database_backup_policies
SET s3_target_id = s3_target_id || ':' || (
  SELECT pr.organization_id
  FROM applications a
  JOIN environments e ON e.id = a.environment_id
  JOIN projects pr ON pr.id = e.project_id
  WHERE a.id = database_backup_policies.application_id
)
WHERE s3_target_id IN (
  SELECT target_id FROM policy_target_orgs GROUP BY target_id HAVING COUNT(*) > 1
);

-- 1. Targets used by exactly one organization.
UPDATE backup_s3_targets
SET organization_id = (
  SELECT o.organization_id FROM policy_target_orgs o WHERE o.target_id = backup_s3_targets.id
)
WHERE is_default = 0 AND organization_id IS NULL
  AND id IN (SELECT target_id FROM policy_target_orgs GROUP BY target_id HAVING COUNT(*) = 1);

-- 3. Unused targets go to the root organization (the operator's own).
UPDATE backup_s3_targets
SET organization_id = (SELECT id FROM organizations WHERE is_root = 1 ORDER BY created_at LIMIT 1)
WHERE is_default = 0 AND organization_id IS NULL
  AND id NOT IN (SELECT target_id FROM policy_target_orgs);

DROP TABLE policy_target_orgs;

-- Names are unique per organization now, not across the whole instance.
CREATE UNIQUE INDEX idx_backup_s3_targets_org_name
  ON backup_s3_targets(COALESCE(organization_id, ''), name);
CREATE INDEX idx_backup_s3_targets_organization ON backup_s3_targets(organization_id);
