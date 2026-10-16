//! Domain (hostname/port/route) management, split by responsibility: `crud`
//! is the DB-backed row CRUD plus sslip.io auto-domain generation; `caddy_sync`
//! pushes domain state into the Caddy config and reconciles it back; `validate`
//! is hostname/port/ingress-mode validation shared by both.

mod caddy_sync;
mod crud;
mod validate;

#[cfg(test)]
mod tests;

pub use caddy_sync::*;
pub use crud::*;
pub use validate::*;
