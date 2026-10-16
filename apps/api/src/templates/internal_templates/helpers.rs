use crate::state::state::SharedState;

/// The resource-detail card shows this next to the image name, so it must track the
/// actual container - not `application_deployments.status`, which only moves when a
/// deploy/start/stop runs or the 45s monitor reconciler catches up. Falls back to that
/// DB status when the live Podman check itself fails (host unreachable), same as the
/// Overview tab's live-status widgets do.
pub(crate) async fn live_deployment_status(
    state: &SharedState,
    application_id: &str,
    fallback: String,
) -> String {
    match crate::deployment::compose_standalone::status(state, application_id).await {
        Ok(services) if !services.is_empty() => {
            let running = services.iter().any(|service| {
                let status = service.status.to_ascii_lowercase();
                status.starts_with("running") || status.starts_with("up")
            });
            if running {
                "running".into()
            } else {
                "stopped".into()
            }
        }
        _ => fallback,
    }
}

/// The top-level `networks:` block for a generated internal database template.
///
/// A database resource sits on one network of its own, which only explicitly
/// linked applications join. It is deliberately NOT on the host-wide
/// `litepod-network`: Compose registers the short service name (`postgres`,
/// `mariadb`, ...) as a DNS alias on every network it joins, so two resources
/// of the same kind both answered to that name there and resolution between
/// them was a coin flip.
///
/// Attaching a domain to a resource still works without this declaring
/// `litepod_proxy`: `generate_standalone_override` always declares that network
/// and joins the routed service to it.
///
/// The logical key and physical name must match what
/// [`crate::deployment::resource_networks::compose_override`] generates. Both
/// declare the same network with the same values, so Compose merges them into a
/// single attachment instead of trying to join the container twice.
pub(crate) fn resource_network_compose(application_id: &str) -> String {
    format!(
        "networks:\n  {key}:\n    external: true\n    name: {name}\n",
        key = crate::deployment::resource_networks::network_key(application_id),
        name = crate::deployment::resource_networks::network_name(application_id),
    )
}

/// The per-service `networks:` block that goes with it, indented for a service
/// under `services:`.
pub(crate) fn resource_service_networks(application_id: &str) -> String {
    format!(
        "    networks:\n      {}: {{}}\n",
        crate::deployment::resource_networks::network_key(application_id)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The template's own Compose file and the override generated at deploy time
    /// both declare this network. If the logical key or the physical name ever
    /// drifted apart, Compose would see two networks instead of one and try to
    /// attach the container twice.
    #[test]
    fn the_template_and_the_deploy_override_name_the_same_network() {
        let application_id = "mariadb1-ab12";
        let key = crate::deployment::resource_networks::network_key(application_id);
        let name = crate::deployment::resource_networks::network_name(application_id);
        assert!(resource_service_networks(application_id).contains(&format!("      {key}: {{}}")));
        assert_eq!(
            resource_network_compose(application_id),
            format!("networks:\n  {key}:\n    external: true\n    name: {name}\n")
        );
    }

    /// The whole point of the resource network: a database is not on the network
    /// every application on the host shares.
    #[test]
    fn a_resource_is_not_put_on_the_shared_network() {
        assert!(!resource_network_compose("db-ab12").contains("litepod-network"));
        assert!(!resource_service_networks("db-ab12").contains("litepod_proxy"));
    }
}
