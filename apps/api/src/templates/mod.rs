pub mod catalog_templates;
pub mod internal_templates;
pub mod secrets;

pub(crate) use internal_templates::{
    live_deployment_status, resource_network_compose, resource_service_networks,
};
