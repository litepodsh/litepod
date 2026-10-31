# Container file explorer — tasks

- [x] Backend module `podman/containers/container_files.rs`: list (`exec ls -1Ap`), read/download
      (`cp` out, size guard via `wc -c`), save (keeps mode/owner), create/upload (no overwrite).
- [x] Handlers `handler/container_files.rs` under `.../containers/{container_id}/files{,/file,/download,/entries,/upload}`;
      permission `app.exec`; container must belong to the application; audit entries.
- [x] UI: `containerFilesWorkspace` client, **Files** button per container (needs `app.exec`, running,
      non-replica), dialog reusing `VolumeFileWorkspace`. i18n en/es.
- [x] Unit tests (path safety, listing order, tar metadata). Verified `ls`, `wc`, `cp` in/out on real Podman (alpine).
- [x] Docs `docs/containers/file-explorer.md`.
- [ ] Test through the API with a real session (list a running app, edit a file, upload, download).
- [ ] Replica containers of group-targeted apps.
- [ ] Router-level test that a foreign container id returns 404.
