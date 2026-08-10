//! Open, value-first capability configuration for Framework agents.
//!
//! These are the neutral `everruns-capability` contract types re-exported at
//! their stable Framework paths (EVE-873). Downstream capability authors can
//! keep depending only on `everruns`, or depend on the dependency-light
//! `everruns-capability` crate directly — both expose the same
//! [`CapabilityRef`]/[`CapabilitySpec`]/[`IntoCapability`] contract, and the
//! same reference representation round-trips through hosted product
//! attachments and worker resolution.
//!
//! # Example
//!
//! ```
//! use everruns::{CapabilityRef, CapabilitySpec, IntoCapability};
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
//! ```

pub use everruns_capability::{CapabilityRef, CapabilitySpec, IntoCapability};
