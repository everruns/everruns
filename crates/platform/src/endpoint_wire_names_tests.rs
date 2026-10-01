//! EVE-1131 renamed the App-era endpoint types in Rust only. Their OpenAPI
//! component names and serde spellings are a public contract (third-party
//! clients and the generated UI types key on them), so pin both here.

use crate::{AgentEndpoint, AppChannel, ChannelType, EndpointTransport};

#[cfg(feature = "openapi")]
#[test]
fn renamed_endpoint_types_keep_app_era_openapi_names() {
    use crate::{
        EndpointAuthConfig, EndpointAuthMode, EndpointAuthProviderConfig, EndpointAuthRequirements,
    };
    use utoipa::ToSchema;

    assert_eq!(AgentEndpoint::name(), "AppChannel");
    assert_eq!(EndpointTransport::name(), "ChannelType");
    assert_eq!(EndpointAuthConfig::name(), "AppEndpointAuthConfig");
    assert_eq!(EndpointAuthMode::name(), "AppEndpointAuthMode");
    assert_eq!(
        EndpointAuthProviderConfig::name(),
        "AppEndpointAuthProviderConfig"
    );
    assert_eq!(
        EndpointAuthRequirements::name(),
        "AppEndpointAuthRequirements"
    );
}

#[test]
fn old_names_are_the_same_types_and_serde_is_unchanged() {
    // The aliases must be re-exports, not copies: a value of one is a value of
    // the other.
    let transport: ChannelType = EndpointTransport::ApiEndpoint;
    assert_eq!(serde_json::to_value(&transport).unwrap(), "api_endpoint");
    assert_eq!(
        serde_json::from_value::<EndpointTransport>("public_chat".into()).unwrap(),
        EndpointTransport::PublicChat
    );

    let endpoint: AgentEndpoint = serde_json::from_value(serde_json::json!({
        "id": "appchan_01933b5a000070008000000000000001",
        "channel_type": "fcp",
        "channel_config": {},
        "enabled": true,
        "status": "live",
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
    }))
    .unwrap();
    let legacy: &AppChannel = &endpoint;
    let wire = serde_json::to_value(legacy).unwrap();
    assert_eq!(wire["id"], "appchan_01933b5a000070008000000000000001");
    assert_eq!(wire["channel_type"], "fcp");
    assert_eq!(wire["status"], "live");
    assert!(wire.get("internal_id").is_none());
}
