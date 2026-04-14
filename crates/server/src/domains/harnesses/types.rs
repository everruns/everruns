// Harness domain types — re-exports from existing locations.
//
// During migration, request types still live in api/harnesses.rs and storage
// row types in storage/models.rs. This module re-exports them so domain
// code has a single import path. Once all callers are migrated, types
// will move here.

pub use crate::api::harnesses::{
    CheckNameQuery, CheckNameResponse, CreateHarnessRequest, HarnessPreviewResponse,
    PreviewHarnessRequest, UpdateHarnessRequest,
};
pub use crate::storage::models::{CreateHarnessRow, HarnessRow, UpdateHarness};
