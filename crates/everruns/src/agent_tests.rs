//! Unit tests for [`super`], the agent builder.
//!
//! Split out of `agent.rs` to keep it under the file-size ratchet.

use std::sync::{Arc, Mutex};

use everruns_provider::tool_types::ToolCall;
use serde_json::{Value, json};

use super::*;
use crate::{FunctionTool, InMemoryEngine};

fn obj_schema() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

#[test]
fn build_rejects_invalid_tool_name() {
    let err = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("ok"))
        .tool(FunctionTool::new(
            "bad name",
            "desc",
            obj_schema(),
            |_: Value| async move { Ok::<_, String>(json!({})) },
        ))
        .build()
        .unwrap_err();
    assert!(
        matches!(err, BuildError::InvalidToolName { ref name, .. } if name == "bad name"),
        "got {err:?}"
    );
}

#[test]
fn build_rejects_invalid_tool_schema() {
    let err = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("ok"))
        .tool(FunctionTool::new(
            "arr",
            "desc",
            json!({ "type": "array" }),
            |_: Value| async move { Ok::<_, String>(json!({})) },
        ))
        .build()
        .unwrap_err();
    assert!(
        matches!(err, BuildError::InvalidToolSchema { ref name, .. } if name == "arr"),
        "got {err:?}"
    );
}

#[test]
fn build_rejects_duplicate_tool_names() {
    let make = || {
        FunctionTool::new("dup", "desc", obj_schema(), |_: Value| async move {
            Ok::<_, String>(json!({}))
        })
    };
    let err = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("ok"))
        .tool(make())
        .tool(make())
        .build()
        .unwrap_err();
    assert_eq!(
        err,
        BuildError::DuplicateTool {
            name: "dup".to_string()
        }
    );
}

#[tokio::test]
async fn function_tool_executes_end_to_end() {
    // Capture what the handler received to prove args flowed in.
    let received: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
    let sink = received.clone();
    let tool = FunctionTool::new(
        "greet",
        "Greet a person by name.",
        json!({
            "type": "object",
            "properties": { "name": { "type": "string" } },
            "required": ["name"],
        }),
        move |args: Value| {
            let sink = sink.clone();
            async move {
                *sink.lock().unwrap() = Some(args.clone());
                let name = args["name"].as_str().unwrap_or("world");
                Ok::<_, String>(json!({ "greeting": format!("Hello, {name}!") }))
            }
        },
    );

    let agent = Agent::builder()
        .instructions("Call greet when asked to greet someone.")
        .model(Model::simulated_scripted(
            "All done.",
            vec![
                vec![ToolCall {
                    id: "call_greet_1".into(),
                    name: "greet".into(),
                    arguments: json!({ "name": "Ada" }),
                }],
                vec![],
            ],
        ))
        .tool(tool)
        .build()
        .expect("valid agent");

    let session = InMemoryEngine::new().create(agent.clone());
    let turn = session.run("Please greet Ada.").await.expect("turn runs");

    assert!(turn.success, "turn should succeed: {:?}", turn.error);
    assert_eq!(turn.tool_calls, 1, "the function tool must have executed");
    assert_eq!(turn.response, "All done.");
    assert_eq!(
        received.lock().unwrap().as_ref().expect("handler ran")["name"],
        json!("Ada"),
        "handler must receive the model's call arguments",
    );
}

#[tokio::test]
async fn function_tool_handler_error_is_redacted_not_a_panic() {
    let tool = FunctionTool::new(
        "always_fails",
        "Always returns an error.",
        obj_schema(),
        |_: Value| async move { Err::<Value, String>("boom".to_string()) },
    );

    let agent = Agent::builder()
        .instructions("Call the tool.")
        .model(Model::simulated_scripted(
            "Handled.",
            vec![
                vec![ToolCall {
                    id: "call_fail_1".into(),
                    name: "always_fails".into(),
                    arguments: json!({}),
                }],
                vec![],
            ],
        ))
        .tool(tool)
        .build()
        .expect("valid agent");

    let session = InMemoryEngine::new().create(agent.clone());
    // The handler error becomes a redacted tool result the model consumes;
    // the turn still completes rather than panicking.
    let turn = session.run("go").await.expect("turn runs");
    assert!(turn.success, "turn should recover from a tool error");
    assert_eq!(turn.tool_calls, 1);
}

#[test]
fn build_rejects_blank_instructions() {
    let err = Agent::builder()
        .instructions("   ")
        .model(Model::simulated("hi"))
        .build()
        .unwrap_err();
    assert_eq!(err, BuildError::BlankInstructions);
}

#[test]
fn build_rejects_missing_model() {
    let err = Agent::builder()
        .instructions("You are concise.")
        .build()
        .unwrap_err();
    assert_eq!(err, BuildError::MissingModel);
}

#[test]
fn build_succeeds_with_simulator() {
    let agent = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("Sure."))
        .name("assistant")
        .build()
        .expect("valid agent");
    assert_eq!(agent.name, "assistant");
}

#[test]
fn build_resolves_plain_model_id_against_provider() {
    let agent = Agent::builder()
        .instructions("You are concise.")
        .provider(Provider::new(
            "acme",
            LlmSimDriver::new(LlmSimConfig::fixed("Sure.")),
        ))
        .model("assistant-v1")
        .build()
        .expect("valid agent");

    assert_eq!(agent.model, ModelSpec::on("acme", "assistant-v1"));
    assert_eq!(agent.provider.id().as_str(), "acme");
}

#[test]
fn build_accepts_a_model_with_its_provider() {
    let provider = Provider::new("acme", LlmSimDriver::new(LlmSimConfig::fixed("Sure.")));
    let agent = Agent::builder()
        .instructions("You are concise.")
        .model(Model::new("assistant-v1", provider))
        .build()
        .expect("valid agent");

    assert_eq!(agent.model, ModelSpec::on("acme", "assistant-v1"));
    assert_eq!(agent.provider.id().as_str(), "acme");
}

#[test]
fn build_rejects_live_model_without_provider() {
    let err = Agent::builder()
        .instructions("You are concise.")
        .model("assistant-v1")
        .build()
        .unwrap_err();

    assert_eq!(err, BuildError::MissingProvider);
}

#[test]
fn build_rejects_multiple_providers() {
    let provider = |id| Provider::new(id, LlmSimDriver::new(LlmSimConfig::fixed("Sure.")));
    let err = Agent::builder()
        .instructions("You are concise.")
        .provider(provider("zeta"))
        .provider(provider("alpha"))
        .model("assistant-v1")
        .build()
        .unwrap_err();

    assert_eq!(
        err,
        BuildError::MultipleProviders {
            registered: vec!["alpha".to_string(), "zeta".to_string()],
        }
    );
}

#[test]
fn workspace_policy_defaults_to_read_only() {
    let agent = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("Sure."))
        .build()
        .expect("valid agent");

    assert!(agent.workspace_policy.permits_read("notes.txt"));
    assert!(!agent.workspace_policy.permits_write("notes.txt"));
    assert!(!agent.workspace_policy.permits_read(".env"));
}

#[tokio::test]
async fn custom_workspace_policy_reaches_the_high_level_runtime() {
    let workspace = tempfile::tempdir().expect("workspace tempdir");
    let policy = crate::WorkspacePolicy::builder()
        .allow_read("public")
        .build()
        .expect("valid policy");
    let agent = Agent::builder()
        .instructions("Read the workspace.")
        .model(Model::simulated("Sure."))
        .workspace(workspace.path())
        .workspace_policy(policy)
        .file("public/note.txt", "visible")
        .file("private/note.txt", "hidden")
        .build()
        .expect("valid agent");

    let session_id = SessionId::new();
    let runtime = agent
        .build_runtime_with_backends(
            HostBackends::in_memory(),
            session_id,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("runtime builds");
    let visible = runtime
        .read_file(session_id, "public/note.txt")
        .await
        .expect("allowed read")
        .expect("seeded file");
    assert_eq!(visible.content.as_deref(), Some("visible"));

    let denied = runtime.read_file(session_id, "private/note.txt").await;
    assert!(denied.is_err());
}

#[cfg(feature = "openai")]
#[test]
fn openai_provider_converts_without_leaking_key() {
    use crate::providers::openai::OpenAI;

    let provider: Provider = OpenAI::new("sk-super-secret").into();
    assert_eq!(provider.id().as_str(), "openai");
    let rendered = format!("{provider:?}");
    assert!(!rendered.contains("sk-super-secret"), "got {rendered}");
}

#[cfg(feature = "openai")]
#[tokio::test]
async fn openai_agent_builds_runtime_offline() {
    use crate::providers::openai::OpenAI;

    // Building the runtime registers the OpenAI driver and assembles the
    // in-process composition without any network call — the provider is only
    // contacted when a turn actually runs, which this test never does.
    let agent = Agent::builder()
        .instructions("You are concise.")
        .provider(OpenAI::new("sk-test"))
        .model("gpt-5-mini")
        .build()
        .expect("valid agent");

    let runtime = agent
        .build_runtime_with_backends(
            HostBackends::in_memory(),
            SessionId::new(),
            None,
            None,
            None,
            None,
        )
        .await
        .expect("openai runtime builds offline");
    let _ = runtime;
}

#[tokio::test]
async fn build_runtime_seeds_the_requested_session_id() {
    let agent = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("Sure."))
        .build()
        .expect("valid agent");

    let session_id = SessionId::new();
    let runtime = agent
        .build_runtime_with_backends(
            HostBackends::in_memory(),
            session_id,
            None,
            None,
            None,
            None,
        )
        .await
        .expect("runtime builds");
    // The seeded session id is usable directly: a caller can run a turn
    // against it without going through `default_session_id`.
    let result = runtime
        .run_turn(session_id, everruns_core::InputMessage::user("hi"))
        .await
        .expect("turn runs");
    assert!(result.success);
    assert_eq!(result.response, "Sure.");
}
