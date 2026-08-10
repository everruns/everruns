//! The neutral Everruns capability contract (EVE-873).
//!
//! One open identity/configuration contract shared by the `everruns`
//! Framework, the hosted product composition, and integration crates:
//!
//! - [`CapabilityId`]: an open, string-based capability identifier with one
//!   validation rule set (character set, length, reserved namespace).
//! - [`CapabilityRef`]: a reference to a capability implementation plus its
//!   per-agent JSON object configuration. This is the persisted attachment
//!   representation and the Framework activation value — the same bytes
//!   round-trip through application code, database rows, and worker
//!   resolution.
//! - [`CapabilitySpec`] and the non-sealed [`IntoCapability`]: the open
//!   conversion seam application values use to become capability
//!   registrations.
//! - [`Definition`] (feature `definition`): the code-defined capability
//!   authoring surface — typed tool handlers, schema metadata, structured
//!   execution errors — with no engine or host dependency.
//! - [`CapabilityIdIndex`] and [`ActivationSet`]: canonical-id/alias
//!   bookkeeping and duplicate/collision rejection shared by registries.
//!
//! This crate deliberately depends on neither `everruns-core` nor
//! `everruns-host`, and carries no Tokio, HTTP, SQLx, OpenAPI, inventory, or
//! platform-record dependency. Third-party capability crates can depend on
//! this crate alone; application authors normally consume the same types
//! re-exported from `everruns`. Part of the [Everruns](https://everruns.com)
//! ecosystem.
//!
//! # Example
//!
//! ```
//! use everruns_capability::{CapabilityRef, CapabilitySpec, IntoCapability};
//! use serde_json::json;
//!
//! struct VendorSearch {
//!     index: String,
//! }
//!
//! impl IntoCapability for VendorSearch {
//!     fn into_capability(self) -> CapabilitySpec {
//!         CapabilityRef::new("vendor.search")
//!             .config(json!({ "index": self.index }))
//!             .into()
//!     }
//! }
//!
//! let spec = VendorSearch { index: "prod".into() }.into_capability();
//! assert_eq!(spec.capability_ref().id(), "vendor.search");
//! ```

#![deny(missing_docs)]

mod error;
mod id;
mod reference;
mod registry;
mod spec;

#[cfg(feature = "definition")]
pub mod definition;

pub use error::CapabilityError;
pub use id::{
    CapabilityId, PLUGIN_CAPABILITY_PREFIX, RESERVED_CAPABILITY_ID_NAMESPACE, is_plugin_capability,
    parse_plugin_capability_id, plugin_capability_id, validate_capability_id,
};
pub use reference::{CapabilityRef, validate_capability_config};
pub use registry::{ActivationSet, CapabilityIdIndex};
pub use spec::{CapabilitySpec, CapabilitySpecParts, IntoCapability};

#[cfg(feature = "definition")]
pub use definition::Definition;

// Dependency re-exports so downstream capability crates can align derive
// macro paths (`#[serde(crate = "...")]`) without adding their own pins.
pub use serde;
pub use serde_json;

#[cfg(feature = "definition")]
pub use async_trait::async_trait;
#[cfg(feature = "definition")]
pub use schemars;
