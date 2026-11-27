//! Provider-owned model-visible tool layout, shared with request budgeting.

/// The tool representation selected for this model, endpoint and request.
/// This describes model input, not cache hits or authorization to execute tools.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ToolRequestProjection {
    /// Only the current tool declarations are sent.
    Current,
    /// The epoch's baseline is sent with inline additions and changes.
    BaselineWithChanges,
    /// An extension has not specified its layout. Reserve both representations
    /// conservatively; never erase the extension's provider-neutral metadata.
    #[default]
    Conservative,
}
