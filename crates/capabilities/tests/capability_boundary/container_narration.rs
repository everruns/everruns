use everruns_capabilities::container_sandbox::ContainerSandboxCapability;
use everruns_contracts::tool_types::ToolCall;
use everruns_core::capabilities::Capability;
use everruns_core::tool_narration::{ToolNarrationContext, ToolNarrationPhase};
use serde_json::json;

#[test]
fn catalog_tools_narrate_all_phases_without_exposing_bodies() {
    let tools = ContainerSandboxCapability.tools();
    assert!(!tools.is_empty());
    for tool in tools {
        let call = ToolCall {
            id: "narration-audit".into(),
            name: tool.name().into(),
            arguments: json!({"name":"SAFE_LABEL", "title":"SAFE_LABEL", "path":"/workspace/SAFE_LABEL", "filename":"SAFE_LABEL", "sprite_name":"SAFE_LABEL", "sandbox_id":"SAFE_LABEL", "model_id":"SAFE_LABEL", "query":"SAFE_LABEL", "repo":"SAFE_LABEL", "image_id":"SAFE_LABEL", "checkpoint_id":"SAFE_LABEL", "prompt":"PRIVATE_BODY", "body":"PRIVATE_BODY", "content":"PRIVATE_BODY", "env_vars":{"KEY":"PRIVATE_BODY"}, "api_key":"PRIVATE_BODY", "token":"PRIVATE_BODY"}),
        };
        for locale in [None, Some("uk-UA")] {
            let mut lines = Vec::new();
            for phase in [
                ToolNarrationPhase::Started,
                ToolNarrationPhase::Waiting,
                ToolNarrationPhase::Completed,
                ToolNarrationPhase::Failed,
            ] {
                let line = tool
                    .narrate(&call, phase, locale, ToolNarrationContext::default())
                    .unwrap_or_else(|| panic!("{} lacks {phase:?} narration", tool.name()));
                assert!(!line.contains("PRIVATE_BODY"), "{line}");
                assert!(!line.contains(tool.name()), "raw tool name: {line}");
                assert!(!line.contains('\n'), "{line}");
                lines.push(line);
            }
            assert_eq!(lines[0], lines[1]);
            assert_ne!(
                lines[0],
                lines[2],
                "{} completion must use past tense",
                tool.name()
            );
            assert_ne!(
                lines[2],
                lines[3],
                "{} failure must not claim success",
                tool.name()
            );
            assert!(
                tool.narrate(
                    &ToolCall {
                        arguments: json!({}),
                        ..call.clone()
                    },
                    ToolNarrationPhase::Completed,
                    locale,
                    ToolNarrationContext::default()
                )
                .is_some()
            );
        }
    }
}
