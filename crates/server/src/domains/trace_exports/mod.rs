//! Customer-owned export configuration. Platform observability stays independent.
mod model;
pub mod router;
pub mod transport;
pub use model::*;

pub mod commands;

pub mod destination_history;

pub mod health;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) mod test_support;
