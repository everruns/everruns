//! A driver's declared variables resolve end to end, and nothing else does.
//!
//! Uses a fixture driver so the contract is tested without a vendor crate:
//! what matters is that resolution is driven by the declaration, whatever a
//! driver declares.

use everruns_provider::credential_provider::{
    EnvCredentialError, EnvCredentialProvider, provider_from_env_with,
};
use everruns_provider::credential_schema::{CredentialFormSchema, FormField};
use everruns_provider::driver_registry::{
    BoxedChatDriver, ChatDriver, DriverConfig, DriverDescriptor, LlmCallConfig, LlmResponseStream,
    Message,
};
use everruns_provider::provider::DriverId;
use everruns_provider::runtime_provider::ProviderEndpoint;

struct Unused;

#[async_trait::async_trait]
impl ChatDriver for Unused {
    async fn chat_completion_stream(
        &self,
        _endpoint: &ProviderEndpoint,
        _messages: Vec<Message>,
        _config: &LlmCallConfig,
    ) -> everruns_provider::Result<LlmResponseStream> {
        unreachable!("the contract never sends a request")
    }
}

fn multi_field_driver() -> DriverDescriptor {
    DriverDescriptor {
        credential_schema: CredentialFormSchema {
            fields: vec![
                FormField::password("access_key_id", "Access Key ID")
                    .required()
                    .env("VENDOR_ACCESS_KEY_ID"),
                FormField::password("secret_access_key", "Secret")
                    .required()
                    .env("VENDOR_SECRET_ACCESS_KEY"),
                FormField::text("region", "Region")
                    .env("VENDOR_REGION")
                    .env_fallback("VENDOR_DEFAULT_REGION"),
            ],
            instructions_markdown: String::new(),
        },
        base_url_env: None,
        ..DriverDescriptor::chat_only(DriverId::external("vendor"), |config: &DriverConfig| {
            // Every declared field arrives typed, exactly as it does when an
            // operator enters the same values in the Settings form.
            assert_eq!(
                config.credentials.get("access_key_id").map(String::as_str),
                Some("AKIA")
            );
            assert_eq!(
                config
                    .credentials
                    .get("secret_access_key")
                    .map(String::as_str),
                Some("shh")
            );
            assert_eq!(
                config.credentials.get("region").map(String::as_str),
                Some("eu-west-1")
            );
            everruns_provider::runtime_provider::Provider::new(config.provider.clone(), Unused)
                .into_boxed_driver()
        })
    }
}

fn lookup(entries: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
    move |name: &str| {
        entries
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.to_string())
    }
}

#[test]
fn a_multi_field_credential_reaches_the_driver_typed() {
    let provider = provider_from_env_with(
        &multi_field_driver(),
        "vendor-primary",
        lookup(&[
            ("VENDOR_ACCESS_KEY_ID", "AKIA"),
            ("VENDOR_SECRET_ACCESS_KEY", "shh"),
            ("VENDOR_DEFAULT_REGION", "eu-west-1"),
        ]),
    )
    .expect("declared variables are set");
    assert_eq!(provider.id().as_str(), "vendor-primary");
}

#[test]
fn a_partial_credential_is_an_error_not_a_broken_provider() {
    // The old id-derived scheme resolved a single `<ID>_API_KEY` or nothing at
    // all; a multi-field driver could never be configured from env. Now a
    // missing required field fails loudly and names what to set.
    let error = provider_from_env_with(
        &multi_field_driver(),
        "vendor",
        lookup(&[("VENDOR_ACCESS_KEY_ID", "AKIA")]),
    )
    .unwrap_err();
    match error {
        EnvCredentialError::Missing { driver, variables } => {
            assert_eq!(driver, "vendor");
            assert!(variables.contains(&"VENDOR_SECRET_ACCESS_KEY".to_string()));
        }
        other => panic!("expected a missing-credential error, got {other:?}"),
    }
}

#[test]
fn an_endpoint_override_alone_is_not_a_credential() {
    // Only the endpoint variable set. Building a provider from that would
    // defer the failure to a 401 at the first request instead of naming the
    // variable to set.
    let descriptor = DriverDescriptor {
        credential_schema: CredentialFormSchema::api_key("VENDOR_API_KEY", ""),
        base_url_env: Some("VENDOR_BASE_URL".into()),
        ..DriverDescriptor::chat_only(DriverId::external("vendor"), |_| -> BoxedChatDriver {
            unreachable!("an endpoint is not a credential")
        })
    };
    let error = provider_from_env_with(
        &descriptor,
        "vendor",
        lookup(&[("VENDOR_BASE_URL", "https://proxy.example")]),
    )
    .unwrap_err();
    assert!(matches!(error, EnvCredentialError::Missing { .. }));
}

#[test]
fn the_driver_id_no_longer_implies_any_variable_name() {
    // `VENDOR_API_KEY` is what the retired scheme would have derived from the
    // id `vendor`. Nothing derives it now, so it configures nothing.
    let declared_nothing = DriverDescriptor {
        credential_schema: CredentialFormSchema {
            fields: vec![FormField::password("api_key", "API Key").required()],
            instructions_markdown: String::new(),
        },
        base_url_env: None,
        ..DriverDescriptor::chat_only(DriverId::external("vendor"), |_| -> BoxedChatDriver {
            unreachable!("no declaration, no credential")
        })
    };
    assert!(
        EnvCredentialProvider::resolve_with(
            &declared_nothing,
            lookup(&[("VENDOR_API_KEY", "key"), ("VENDOR_BASE_URL", "https://x")]),
        )
        .is_none()
    );
}
