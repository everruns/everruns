//! Worker with a sealed Bashkit primary Sandbox.

use crate::domains::harnesses::record::BuiltInHarnessDefinition;
use everruns_contracts::capability::BuiltInHarnessPreset;

pub fn definition() -> BuiltInHarnessDefinition {
    super::levels::definition(BuiltInHarnessPreset::BashkitWorker)
}
