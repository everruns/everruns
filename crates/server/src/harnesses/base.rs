//! Base retains the same blank foundation on both surfaces.
use crate::domains::harnesses::record::BuiltInHarnessDefinition;
pub fn definition() -> BuiltInHarnessDefinition {
    super::levels::definition(everruns_contracts::capability::BuiltInHarnessPreset::Base)
}
