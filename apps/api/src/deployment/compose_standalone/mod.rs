//! Standalone (non-multi-tenant-network) Compose deployment pipeline, split by
//! responsibility: `overrides` builds the YAML litepod layers on top of the
//! user's Compose file, `process` shells out to `podman-compose` and streams
//! its output, `pipeline` is the deploy orchestrator that wires the other two
//! together with the job queue and domain routing, and `lifecycle` covers
//! start/stop/remove/status for an already-deployed app. `shared` holds the
//! handful of tiny helpers (`project_name`, `now_ms`, `tail`) all four call.

mod lifecycle;
mod overrides;
mod pipeline;
mod process;
mod shared;

#[cfg(test)]
mod tests;

pub(crate) use lifecycle::json_string;
pub use lifecycle::{remove, start, status, stop};

pub(crate) use pipeline::run;
pub use pipeline::{enqueue, enqueue_rollback};

pub(crate) use overrides::{
    prepare_remote_compose, service_exposes_port, service_for_port, services_for_port,
};

pub(crate) use process::teardown_project;
