//! Guard for the hidden maintenance companion (`applications.maintenance_for`).
//!
//! A companion is a real row in `applications`, so any query that lists
//! applications would show it, count it, bill it or route to it unless it filters
//! it out. This test reads the sources and fails on a query over `applications`
//! that does none of:
//!
//! - filter with `maintenance_for IS NULL`;
//! - go by a single id (`id = ?`), which cannot list anything;
//! - appear in [`ALLOWED`] with the reason it must see companions.
//!
//! Adding a new listing without the filter fails here, with the file and line.

use std::path::{Path, PathBuf};

/// `(file, text found in the statement, why companions are meant to be seen)`.
const ALLOWED: &[(&str, &str, &str)] = &[
    ("deployment/compose/store.rs", "SELECT id FROM applications", "host-port reservation must see every live application, companions included"),
    ("deployment/deploy_hook.rs", "WHERE deploy_token = ?1", "a lookup by unique token; a companion has none"),
    ("deployment/domains/caddy_sync.rs", "LEFT JOIN applications a ON a.id = d.application_id", "starts from domains; a companion has none"),
    ("deployment/domains/validate.rs", "FROM application_domains d", "starts from domains; a companion has none"),
    ("metrics_history/requests.rs", "FROM application_domains d", "starts from domains; a companion has none"),
    ("servers/ingress.rs", "FROM application_domains domains", "starts from domains; a companion has none"),
    ("servers/ingress.rs", "FROM application_deployments d", "only collects server ids to reconcile; a companion adds none its parent's server lacks"),
    ("deployment/queue.rs", "const JOB_JOINS", "the build queue shows the companion's build, named \"Maintenance page · <app>\", so a stuck page build is visible"),
    ("assistant/tools.rs", "JOIN applications a ON a.id = d.application_id", "a map keyed by application id, only read for applications that are listed"),
    ("deployment/domains/caddy_sync.rs", "SELECT id, name FROM applications WHERE server_id", "names for edge routes, which are built from domains a companion lacks"),
    ("deployment/resource_networks.rs", "INNER JOIN applications consumer", "starts from network links, which a companion has none of"),
    ("deployment/rollout.rs", "WHERE d.application_id = ?1", "by one application id"),
    ("metrics_history/mod.rs", "JOIN applications a ON a.id", "container ownership for sampling; a companion's container is sampled like any other"),
    ("servers/groups.rs", "JOIN applications a ON a.id = r.application_id", "starts from replicas of server groups, which a companion has none"),
    ("servers/ingress.rs", "SELECT id FROM applications WHERE server_id = ?1 LIMIT 1", "only asks whether any application is on the server"),
    ("servers/monitor.rs", "JOIN applications a ON a.id = d.application_id", "reconciles every container Podman reports, the companion's included"),
    ("servers/boot_restore.rs", "JOIN applications a ON a.id = d.application_id", "restores by policy; a parked companion is desired=stopped and stays down"),
    ("servers/repository.rs", "SELECT COUNT(*) FROM applications WHERE server_id = ?1", "a server with a companion still hosts its parent, so it is never empty"),
    ("notifications/repository.rs", "JOIN applications a ON a.id = r.application_id", "a failed page build is worth a notification, named \"Maintenance page · <app>\""),
    ("git/git_providers/provider.rs", "WHERE git_provider_id = ?1", "a companion is built from an upload and has no git provider"),
    ("git/repositories/project.rs", "SELECT a.id FROM applications a", "teardown lists every id, companions included, to remove their containers"),
    ("servers/repository.rs", "FROM applications WHERE server_id = 'local'", "a local companion would strand when the local server is removed, just like an application"),
];

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

fn is_test_file(relative: &str) -> bool {
    relative.ends_with("tests.rs")
        || relative.contains("/tests/")
        || relative.ends_with("_guard.rs")
        || relative.starts_with("migrations/")
        || relative.starts_with("testing")
}

/// The statement around line `at`: from the line that opens the string to the
/// line that closes it, so a multi-line `SELECT` is judged as one.
fn statement(lines: &[&str], at: usize) -> String {
    let mut start = at;
    while start > 0 && !lines[start].contains("\"") {
        start -= 1;
    }
    let mut end = at;
    while end + 1 < lines.len() && end < at + 8 && !lines[end].contains("\",") && !lines[end].trim_end().ends_with("\")") {
        end += 1;
    }
    lines[start..=end].join("\n")
}

/// `id = ?` on the applications table itself: `a.id = ?`, `applications.id = ?` or a
/// bare `id = ?`. A longer column (`organization_id = ?`) is not one.
fn by_single_id(statement: &str) -> bool {
    // Only the spaces around `=` go, so a word boundary before `id` survives.
    let compact = statement.replace(" = ", "=").replace(" =", "=").replace("= ", "=");
    let bytes = compact.as_bytes();
    let mut from = 0;
    while let Some(found) = compact[from..].find("id=?") {
        let at = from + found;
        let before = &compact[..at];
        let boundary = match before.chars().last() {
            None => true,
            Some('.') => before.ends_with("a.") || before.ends_with("applications."),
            Some(ch) => !(ch.is_ascii_alphanumeric() || ch == '_'),
        };
        if boundary {
            return true;
        }
        from = at + 4;
        if from >= bytes.len() {
            break;
        }
    }
    false
}

#[test]
fn every_listing_of_applications_hides_the_maintenance_companion() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&root, &mut files);

    let mut offenders = Vec::new();
    for file in files {
        let relative = file.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
        if is_test_file(&relative) {
            continue;
        }
        let text = std::fs::read_to_string(&file).unwrap();
        // Unit tests live in the same file; only what precedes `#[cfg(test)]` is production code.
        let production = text.split("#[cfg(test)]").next().unwrap();
        let lines: Vec<&str> = production.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            let upper = line.to_ascii_uppercase();
            if !(upper.contains("FROM APPLICATIONS") || upper.contains("JOIN APPLICATIONS")) {
                continue;
            }
            let stmt = statement(&lines, index);
            let allowed = ALLOWED
                .iter()
                .any(|(path, needle, _)| relative.starts_with(path) && stmt.contains(needle));
            if stmt.contains("maintenance_for") || by_single_id(&stmt) || allowed {
                continue;
            }
            offenders.push(format!("{relative}:{}  {}", index + 1, line.trim()));
        }
    }
    assert!(
        offenders.is_empty(),
        "queries over `applications` that neither filter `maintenance_for IS NULL`, go by one id, nor are allowed with a reason:\n{}",
        offenders.join("\n")
    );
}
