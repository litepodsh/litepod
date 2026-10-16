//! The assistant.
//!
//! Three HTTP routes, not one per capability: send a message, read a
//! conversation, settle an action. What the assistant can do is a tool list
//! (`tools`), not a route table, so adding a capability never adds an endpoint.
//!
//! Tools come in two kinds, and the difference is whether a human has to say
//! yes first:
//!
//! - **read** tools run here, against the same query layer the UI uses, with
//!   the caller's own permissions, and never return a secret's value;
//! - **write** tools do not write. They validate their arguments and store an
//!   [`actions::ActionDto`]; applying it is a separate authenticated request.
//!
//! That last rule is the whole safety model. A tool the model can call is a
//! tool the model can call, so if the write happened inside the handler, one
//! instruction hidden in something the assistant read would be enough to change
//! a project. The confirm carries the user's session and is checked against
//! their role again.
//!
//! The model is reached through whichever connection the organization set up;
//! see [`connections`] for the keys and [`config`] for the provider table.
pub mod actions;
pub mod capabilities;
pub mod config;
pub mod connections;
pub mod conversation;
pub mod endpoint;
pub mod history;
pub mod hooks;
pub mod providers;
pub mod redaction;
pub mod service;
pub mod tools;
