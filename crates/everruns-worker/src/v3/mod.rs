// V3 Minimal forever-running workflow example
//
// Demonstrates the simplest possible infinite loop workflow using temporal-sdk-core.
// The workflow: timer -> activity -> repeat (with continue-as-new after N iterations)

mod workflow;
mod worker;

pub use workflow::*;
pub use worker::*;
