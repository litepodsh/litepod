//! Named permissions, evaluated per request.

pub mod extract;
pub mod guard;
pub mod permission;
pub mod set;

#[cfg(test)]
mod regression;

pub use permission::Permission;
pub use set::{PermissionSet, Target, Visible};
