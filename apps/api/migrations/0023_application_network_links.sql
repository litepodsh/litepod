-- One row per (consumer application, internal database resource) pair the user
-- has linked. A link means the consumer's Compose services join the resource's
-- own Podman network (`<resource_application_id>_db`) at deploy time, so a
-- database is reachable only by the applications explicitly wired to it
-- instead of by everything sharing the host-wide `litepod-network`.
--
-- Both sides cascade: deleting either application drops the link, and the
-- resource's network is torn down with the resource itself.
CREATE TABLE application_network_links (
    id                      TEXT PRIMARY KEY,
    application_id          TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    resource_application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    created_at              INTEGER NOT NULL
);
CREATE UNIQUE INDEX idx_application_network_links_pair
    ON application_network_links(application_id, resource_application_id);
CREATE INDEX idx_application_network_links_resource
    ON application_network_links(resource_application_id);
