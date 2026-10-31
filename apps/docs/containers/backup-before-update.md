# Backup before updating Litepod

**Settings → Backups → Schedule → Back up before updating.** When on, applying a Litepod update
first saves an instance backup to the default S3 storage, then installs.

- The backup starts the moment you press **Update**. If S3 is not configured, or another backup or
  restore is running, the update is refused right away and nothing changes.
- The update only continues when the backup finished successfully. If it fails (or takes longer
  than 30 minutes) the update is **cancelled**, nothing is installed, and the reason shows in the
  update status note (`update cancelled: backup before update failed: ...`).
- The backup is recorded in the backup history like a manual one.
- The panel shows "Backing up before updating" while it runs. The container is replaced only after
  the backup ends.
- Off by default. The option is stored with the default storage target, so saving the storage
  form keeps it.
