//! Changes the assistant wants to make: stored when it asks, applied only when
//! a person confirms.
//!
//! The first write tool is `set_environment_variable`. Proposing validates the
//! arguments and writes a row; confirming replays those arguments through the
//! same service call the environment editor uses. Nothing between the two
//! touches the application.
//!
//! Split by responsibility: `types` holds the action-kind constants, the
//! per-action argument structs, and `ActionDto`/`ActionTarget`; `propose`
//! validates a tool call and stores it as a pending action; `lifecycle` is the
//! store/list/confirm/reject engine; `apply` replays a confirmed action's
//! arguments through the real service call.

mod apply;
mod lifecycle;
mod propose;
mod types;

#[cfg(test)]
mod tests;

pub use types::{
    ActionDto, ActionTarget, CreateApplication, DomainFields, ProjectFields, SetComposeFile,
    SetEnvironmentVariable, SetSource, UpdateApplication,
};

pub use propose::{
    propose_add_domain, propose_compose_file, propose_create_application, propose_create_project,
    propose_delete_domain, propose_delete_environment_variable, propose_delete_project,
    propose_environment_variable, propose_lifecycle, propose_redeploy, propose_source, propose_update_application,
    propose_update_domain, propose_update_project,
};

pub use types::{START_APPLICATION, STOP_APPLICATION};

pub use lifecycle::{confirm, list_for_conversation};

pub use apply::reject;
