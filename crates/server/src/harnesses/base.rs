//! Base retains the same blank foundation on both surfaces.
use crate::records::BuiltInHarnessDefinition;
pub fn definition() -> BuiltInHarnessDefinition {
    super::levels::definition(everruns_contracts::capability::BuiltInHarnessPreset::Base)
}
