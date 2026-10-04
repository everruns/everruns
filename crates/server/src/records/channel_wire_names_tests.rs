//! EVE-1131 renamed the App-era endpoint types in Rust only. Their OpenAPI
//! component names and serde spellings are a public contract (third-party
//! clients and the generated UI types key on them), so pin both here.

use crate::{AgentChannel, ChannelType};

#[test]
fn renamed_endpoint_types_use_channel_openapi_names() {
    use crate::{
        ChannelAuthConfig, ChannelAuthMode, ChannelAuthProviderConfig, ChannelAuthRequirements,
    };
    use utoipa::ToSchema;

    assert_eq!(AgentChannel::name(), "AgentChannel");
    assert_eq!(ChannelType::name(), "ChannelType");
    assert_eq!(ChannelAuthConfig::name(), "ChannelAuthConfig");
    assert_eq!(ChannelAuthMode::name(), "ChannelAuthMode");
    assert_eq!(
        ChannelAuthProviderConfig::name(),
        "ChannelAuthProviderConfig"
    );
    assert_eq!(ChannelAuthRequirements::name(), "ChannelAuthRequirements");
}

#[test]
fn renamed_endpoint_types_keep_app_era_serde_shape() {
    let transport = ChannelType::ApiEndpoint;
    assert_eq!(serde_json::to_value(transport).unwrap(), "api_endpoint");
    assert_eq!(
        serde_json::from_value::<ChannelType>("public_chat".into()).unwrap(),
        ChannelType::PublicChat
    );

    // The endpoint ID keeps its `appchan_` wire prefix.
    let endpoint: AgentChannel = serde_json::from_value(serde_json::json!({
        "id": "appchan_01933b5a000070008000000000000001",
        "channel_type": "fcp",
        "channel_config": {},
        "enabled": true,
        "status": "live",
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
    }))
    .unwrap();
    let wire = serde_json::to_value(&endpoint).unwrap();
    assert_eq!(wire["id"], "appchan_01933b5a000070008000000000000001");
    assert_eq!(wire["channel_type"], "fcp");
    assert_eq!(wire["status"], "live");
    assert!(wire.get("internal_id").is_none());
}
