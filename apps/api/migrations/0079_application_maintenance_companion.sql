-- A maintenance page built by Railpack lives in a hidden "companion" application:
-- the build, run and health machinery is keyed by an `applications` row, so the
-- page needs one. `maintenance_for` names the application it is the page of.
--
-- NULL (every existing row) means an ordinary application. A companion is never
-- listed, counted, or routed on its own; it goes with its parent
-- (ON DELETE CASCADE) and there is at most one per parent.
ALTER TABLE applications ADD COLUMN maintenance_for TEXT REFERENCES applications(id) ON DELETE CASCADE;

CREATE UNIQUE INDEX idx_applications_maintenance_for
    ON applications(maintenance_for) WHERE maintenance_for IS NOT NULL;

-- A companion of a companion would be a page of a page: nothing builds or routes
-- one, and every "hide companions" filter assumes depth one.
CREATE TRIGGER applications_no_nested_companion
BEFORE INSERT ON applications
WHEN NEW.maintenance_for IS NOT NULL
 AND (SELECT maintenance_for FROM applications WHERE id = NEW.maintenance_for) IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'a maintenance companion cannot have a companion');
END;
