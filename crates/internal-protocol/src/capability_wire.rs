//! Wire encoding for per-agent capability configs.
//!
//! Proto carries each `{"ref": id, "config": {…}}` attachment as a JSON string
//! alongside the plain `capability_ids` list. Both harness and agent
//! conversions encode it the same way, so the invariant lives here once.

use serde::Serialize;

/// Encode per-agent capability configs for the proto `capabilities` field.
///
/// Unreachable: `CapabilityRef` is a `String` plus a `serde_json::Value`,
/// neither of which can fail to serialize. Kept as a panic because the
/// reverse conversion rebuilds configs from `capability_ids` with
/// `config: {}` whenever this list is empty, so any non-panicking fallback
/// would silently replace a per-agent capability config with the default.
#[expect(
    clippy::expect_used,
    reason = "fail closed rather than downgrade a capability config on the wire"
)]
pub(crate) fn encode_configs<T: Serialize>(configs: &[T]) -> Vec<String> {
    configs
        .iter()
        .map(|config| serde_json::to_string(config).expect("capability config serializes"))
        .collect()
}
