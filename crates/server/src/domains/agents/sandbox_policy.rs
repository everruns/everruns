//! Agent Sandbox policy validation and storage projections.

use crate::domains::common::CommandError;
use crate::domains::sandbox_templates::record::SandboxPolicy;
use crate::storage::UpdateField;
use serde_json::Value;

pub(super) fn validate(value: Option<&SandboxPolicy>) -> Result<(), CommandError> {
    value.map_or(Ok(()), |set| {
        crate::domains::sandbox_templates::resolution::validate_sandbox_policy(set)
            .map_err(CommandError::unprocessable)
    })
}

pub(super) fn validate_update(value: &UpdateField<SandboxPolicy>) -> Result<(), CommandError> {
    match value {
        UpdateField::Set(set) => validate(Some(set)),
        UpdateField::Unchanged | UpdateField::Clear => Ok(()),
    }
}

pub(super) fn to_json(value: Option<&SandboxPolicy>) -> Option<Value> {
    value.map(|set| serde_json::to_value(set).unwrap_or_default())
}

pub(super) fn update_to_json(value: UpdateField<SandboxPolicy>) -> Option<Option<Value>> {
    match value {
        UpdateField::Unchanged => None,
        UpdateField::Clear => Some(None),
        UpdateField::Set(set) => Some(Some(serde_json::to_value(set).unwrap_or_default())),
    }
}

pub(crate) fn selection_update(value: Option<SandboxPolicy>) -> UpdateField<SandboxPolicy> {
    value.map_or(UpdateField::Unchanged, UpdateField::Set)
}
