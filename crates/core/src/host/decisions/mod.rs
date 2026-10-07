//! Decision drivers on the host side: the registry, the router every caller
//! sees as its `DecisionsService`, and the vendor-free `llm` driver.
//!
//! Vendor drivers live with their vendor (`everruns-integrations` feature `typesafe`
//! for TypeSafe), and the deployment composes them into a registry from
//! above, the same direction the TypeSafe service already took: host never
//! learns a vendor.

mod llm;
mod registry;

pub use llm::{LLM_DECISION_DRIVER_ID, LlmDecisionDriver};
pub use registry::DecisionRouter;
