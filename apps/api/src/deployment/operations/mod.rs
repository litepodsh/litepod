//! Deployment-lifecycle operations, split by responsibility: `resources` is
//! CPU/memory limit math and the container builder both other files share;
//! `identity` resolves what an application is and where it runs; `artifacts`
//! is uploaded build output storage; `environment` is env var CRUD;
//! `caddy` edits the Caddy config file for domains and basic-auth; `queue_ops`
//! enqueues deploy/rollback jobs; `lifecycle` is start/stop/remove/volumes.

mod artifacts;
mod caddy;
mod environment;
mod identity;
mod lifecycle;
mod queue_ops;
mod resources;

#[cfg(test)]
mod tests;

pub use artifacts::*;
pub use caddy::*;
pub use environment::*;
pub use identity::*;
pub use lifecycle::*;
pub use queue_ops::*;
pub(crate) use resources::*;
