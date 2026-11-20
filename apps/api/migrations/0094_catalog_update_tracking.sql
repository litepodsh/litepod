-- What a catalog template looked like when an application was created from it
-- (or last updated), so a later version can be diffed against the template and
-- against the user's own edits. All NULL for applications created before this
-- migration and for applications that never came from the catalog.
ALTER TABLE applications ADD COLUMN catalog_template_version TEXT;
ALTER TABLE applications ADD COLUMN catalog_base_compose TEXT;
ALTER TABLE applications ADD COLUMN catalog_base_env TEXT;
-- A template version the user chose to skip; the update notice stays hidden
-- until the catalog publishes a different one.
ALTER TABLE applications ADD COLUMN catalog_ignored_version TEXT;
