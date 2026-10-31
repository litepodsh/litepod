# Container file explorer

Browse, read, edit, create, upload and download files inside a running container, from the
**Containers** tab: press **Files** on a container.

## Who can use it

Only roles with the `app.exec` permission (the same one the terminal needs). Reading a container's
files exposes secrets and config just as a shell would, so it is not given to viewers or plain
editors by default. Every save, create, upload and download is written to the audit log
(`container.file_updated`, `container.file_created`, `container.file_uploaded`,
`container.file_downloaded`) with the container and path.

## What it does

- Lists a folder with `podman exec <container> ls`. The image therefore needs `ls`; images without
  it (distroless) show an error instead of a tree.
- Reads and writes with `podman cp`, which moves a tar stream and needs no shell. Saving keeps the
  file's mode and owner.
- Creating or uploading never overwrites: if the path exists the request is refused.
- Files over 25 MB are not opened or written from the explorer. A folder shows its first 5000
  entries.
- Paths are relative to `/` and cannot contain `..`. The container is the boundary; the explorer
  can reach anything its processes can.

## Limits

- The container must be running. The button is disabled otherwise.
- Changes live in the container's writable layer and are lost when the container is recreated (a
  redeploy). Use a volume for data you want to keep.
- A read-only root filesystem refuses writes; the error from Podman is shown.
- Containers of group-targeted applications (replicas) are not browsable yet, same as the terminal.
