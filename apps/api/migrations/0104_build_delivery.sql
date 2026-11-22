-- How a build server hands the image to the server that runs the application:
-- `transfer` copies it straight across (podman save | podman load, no registry
-- needed); `registry` pushes to `build_image_repository` and pulls by digest.
ALTER TABLE applications ADD COLUMN build_delivery TEXT NOT NULL DEFAULT 'transfer' CHECK (build_delivery IN ('transfer', 'registry'));

-- Build servers set up before this column already publish to a registry.
UPDATE applications SET build_delivery = 'registry' WHERE build_image_repository IS NOT NULL;
