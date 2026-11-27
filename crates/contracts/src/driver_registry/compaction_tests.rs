use super::*;

struct PureProjection;
#[async_trait]
impl ChatDriver for PureProjection {
    fn supports_history_summarization(&self) -> bool {
        false
    }
    fn tool_request_projection(
        &self,
        endpoint: &crate::ProviderEndpoint,
        _: &[Message],
        config: &LlmCallConfig,
    ) -> ToolRequestProjection {
        assert_eq!(endpoint.base_url(), Some("https://configured.example/v1"));
        assert!(
            config
                .extra_headers
                .iter()
                .any(|(key, value)| key == "x-plan" && value == "configured")
        );
        ToolRequestProjection::Current
    }
    async fn chat_completion_stream(
        &self,
        _: &crate::ProviderEndpoint,
        _: Vec<Message>,
        _: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        panic!("request planning must not dispatch or summarize")
    }
}

#[test]
fn transparent_wrappers_keep_projection_configuration_and_summarization_refusal() {
    let options = crate::provider::ProviderRequestOptions {
        headers: vec![crate::provider::ProviderRequestHeader {
            name: "x-plan".into(),
            value: "configured".into(),
        }],
        ..Default::default()
    };
    let driver = RequestOptionsDriver::wrap(Box::new(PureProjection), &options);
    let driver: BoxedChatDriver = Box::new(CredentialGateDriver {
        inner: driver,
        message: "configure credentials".into(),
    });
    let bound = crate::Provider::new("application-key", driver)
        .base_url("https://configured.example/v1")
        .into_boxed_driver();
    assert!(
        !bound.supports_history_summarization(),
        "wrappers cannot enable side-effecting summary calls"
    );
    let config = LlmCallConfig::new("application-model");
    assert_eq!(
        bound.tool_request_projection(&crate::ProviderEndpoint::default(), &[], &config),
        ToolRequestProjection::Current
    );
    assert!(
        config.extra_headers.is_empty(),
        "planning must not mutate caller config"
    );
}

#[test]
fn a_custom_driver_that_does_not_opt_in_keeps_conservative_accounting() {
    struct DefaultProjection;
    #[async_trait]
    impl ChatDriver for DefaultProjection {
        async fn chat_completion_stream(
            &self,
            _: &crate::ProviderEndpoint,
            _: Vec<Message>,
            _: &LlmCallConfig,
        ) -> Result<LlmResponseStream> {
            unreachable!("planning only")
        }
    }
    let driver = DefaultProjection;
    assert_eq!(
        driver.tool_request_projection(
            &crate::ProviderEndpoint::default(),
            &[],
            &LlmCallConfig::new("custom")
        ),
        ToolRequestProjection::Conservative
    );
}
