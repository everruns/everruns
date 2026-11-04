//! Daytona OpenAPI specification mounted for the `daytona_api_call` tool.
//!
//! Decision: generated at build time from the same vendored specs as the
//! client (`openapi/daytona/`, see `build.rs`), so the endpoints an agent reads
//! about are the ones Daytona serves today rather than a hand-kept copy. The
//! spec is always mounted into the session filesystem at
//! `/daytona/openapi.yaml` (regardless of `enable_api_calling` config); it is
//! JSON, which every YAML reader also accepts.

/// Daytona Management and Toolbox endpoints for agent automation.
pub const DAYTONA_OPENAPI_SPEC: &str =
    include_str!(concat!(env!("OUT_DIR"), "/daytona_agent_openapi.json"));
