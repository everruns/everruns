//! Decision drivers on the host side: the registry, the router every caller
//! sees as its `DecisionsService`, and the vendor-free `llm` driver.
//!
//! Vendor drivers live with their vendor (`everruns-integrations-typesafe`
//! for TypeSafe), and the deployment composes them into a registry from
//! above, the same direction the TypeSafe service already took: host never
//! learns a vendor. See `knowledge/operations/decisions-service.md`.

mod llm;
mod registry;

pub use llm::{LLM_DECISION_DRIVER_ID, LlmDecisionDriver};
pub use registry::{DecisionDriverRegistry, DecisionRouter, DecisionRoutingError};
