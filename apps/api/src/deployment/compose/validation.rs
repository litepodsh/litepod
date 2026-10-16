/// Compose service names that would shadow a control-plane container on the
/// shared proxy network.
///
/// Compose registers every service name as a DNS alias on each network the
/// service joins, and an override can only *add* aliases - never drop that
/// default one. A deployed app with a service named `libsql` therefore answers
/// to the same name the API dials for its own database, and roughly half the
/// control-plane queries land on the tenant's sqld, which rejects the API's JWT.
///
/// Control-plane traffic now uses the `litepod-*` container names, which no
/// service name can produce; this list stays as the second line of defence.
pub const RESERVED_SERVICE_NAMES: [&str; 6] =
    ["api", "app", "buildkit", "caddy", "dragonfly", "libsql"];

/// Reserved names among `names`, in the order given, without duplicates.
pub fn reserved_service_conflicts<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut conflicts: Vec<String> = Vec::new();
    for name in names {
        let name = name.trim().to_ascii_lowercase();
        if RESERVED_SERVICE_NAMES.contains(&name.as_str()) && !conflicts.contains(&name) {
            conflicts.push(name);
        }
    }
    conflicts
}

/// Reserved service names declared by a Compose document. An unparseable file
/// yields none - the deploy and validation paths report the syntax error on
/// their own.
pub fn reserved_services_in_compose(content: &str) -> Vec<String> {
    let Ok(document) = serde_yaml::from_str::<serde_yaml::Value>(content) else {
        return Vec::new();
    };
    let Some(services) = document
        .get("services")
        .and_then(serde_yaml::Value::as_mapping)
    else {
        return Vec::new();
    };
    reserved_service_conflicts(services.keys().filter_map(serde_yaml::Value::as_str))
}

/// User-facing explanation for `conflicts`, which must not be empty.
pub fn reserved_service_message(conflicts: &[String]) -> String {
    let names = conflicts
        .iter()
        .map(|name| format!("'{name}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let subject = if conflicts.len() == 1 {
        "service name is"
    } else {
        "service names are"
    };
    format!(
        "Compose {subject} reserved by litepod's own containers on the shared network: {names}. \
         Rename the service, for example 'web' instead of 'app'."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_reserved_services_case_insensitively() {
        let content = "services:\n  LibSQL:\n    image: x\n  web:\n    image: y\n";
        assert_eq!(reserved_services_in_compose(content), vec!["libsql"]);
    }

    #[test]
    fn allows_a_file_without_reserved_names() {
        let content = "services:\n  web:\n    image: x\n  postgres:\n    image: y\n";
        assert!(reserved_services_in_compose(content).is_empty());
    }

    #[test]
    fn reports_every_conflict_once() {
        let content = "services:\n  app:\n    image: x\n  caddy:\n    image: y\n";
        let conflicts = reserved_services_in_compose(content);
        assert_eq!(conflicts, vec!["app", "caddy"]);
        let message = reserved_service_message(&conflicts);
        assert!(message.contains("'app'"), "{message}");
        assert!(message.contains("'caddy'"), "{message}");
        assert!(message.contains("service names are"), "{message}");
    }

    #[test]
    fn ignores_an_unparseable_file() {
        assert!(reserved_services_in_compose(": : :").is_empty());
    }
}
