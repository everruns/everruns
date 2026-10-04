//! Agent Environment profile validation and storage projections.

use crate::domains::common::CommandError;
use crate::records::EnvironmentSet;
use everruns_durable::UpdateField;
use serde_json::Value;

pub(super) fn validate(value: Option<&EnvironmentSet>) -> Result<(), CommandError> {
    value.map_or(Ok(()), |set| {
        crate::domains::environments::profiles::validate_environment_set(set)
            .map_err(CommandError::unprocessable)
    })
}

pub(super) fn validate_update(value: &UpdateField<EnvironmentSet>) -> Result<(), CommandError> {
    match value {
        UpdateField::Set(set) => validate(Some(set)),
        UpdateField::Unchanged | UpdateField::Clear => Ok(()),
    }
}

pub(super) fn to_json(value: Option<&EnvironmentSet>) -> Option<Value> {
    value.map(|set| serde_json::to_value(set).unwrap_or_default())
}

pub(super) fn update_to_json(value: UpdateField<EnvironmentSet>) -> Option<Option<Value>> {
    match value {
        UpdateField::Unchanged => None,
        UpdateField::Clear => Some(None),
        UpdateField::Set(set) => Some(Some(serde_json::to_value(set).unwrap_or_default())),
    }
}

pub(crate) fn selection_update(value: Option<EnvironmentSet>) -> UpdateField<EnvironmentSet> {
    value.map_or(UpdateField::Unchanged, UpdateField::Set)
}
