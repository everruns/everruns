//! Live tests for OpenAI hosted tools through the Responses driver (EVE-1115)
//! against the real OpenAI API.
//!
//! Proves OpenAI accepts each rendered hosted tool, runs it inside the
//! response, and that the stream reports the call as hosted-call progress plus
//! a per-call count in the completion metadata, never as an agent tool call.
//! File search creates its own vector store for the run and deletes it after.

use everruns_contracts::driver_registry::{
    HostedToolCallStatus, LlmCallConfig, LlmStreamEvent, Message, MessageRole,
};
use everruns_contracts::model::ReasoningEffort;
use everruns_contracts::openai_hosted_tools::{
    ContainerTool, FileSearchTool, McpServerTool, OPENAI_MCP_APPROVAL_TOOL, OpenAiHostedTools,
    SearchContextSize, WebSearchTool,
};
use everruns_contracts::tool_types::ToolCall;
use everruns_drivers::openai::provider;
use futures::StreamExt;

const LIVE_MODEL: &str = "gpt-6-luna";

/// Stream one prompt with `tools` and check the named hosted call completed.
async fn assert_hosted_call(tools: OpenAiHostedTools, prompt: &str, tool: &str, item: &str) {
    let api_key =
        std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set for the live test");
    let mut config = LlmCallConfig::new(LIVE_MODEL);
    config.reasoning_effort = Some(ReasoningEffort::Low);
    let (key, value) = tools.to_driver_option().unwrap();
    config.driver_options.insert(key, value);

    let messages = vec![Message::text(MessageRole::User, prompt)];
    let mut stream = provider("openai", api_key)
        .chat_completion_stream(messages, &config)
        .await
        .expect("OpenAI should accept the hosted tool");

    let (mut text, mut hosted, mut done) = (String::new(), Vec::new(), None);
    while let Some(event) = stream.next().await {
        match event.expect("stream should not fail") {
            LlmStreamEvent::TextDelta(delta) => text.push_str(&delta),
            LlmStreamEvent::HostedToolCall(call) => hosted.push(call),
            LlmStreamEvent::ToolCalls(calls) => panic!("hosted call surfaced as {calls:?}"),
            LlmStreamEvent::Error(error) => panic!("stream error: {error:?}"),
            LlmStreamEvent::Done(meta) => done = Some(meta),
            _ => {}
        }
    }
    let done = done.expect("stream completes");
    eprintln!(
        "answer: {text}\nhosted: {hosted:?}\ncounts: {:?}",
        done.hosted_tool_calls
    );

    assert!(!text.trim().is_empty(), "model should answer");
    assert!(
        hosted
            .iter()
            .any(|call| call.tool == tool && call.status == HostedToolCallStatus::Completed),
        "expected a completed {tool} call, got {hosted:?}"
    );
    assert!(done.hosted_tool_calls.get(item).copied().unwrap_or(0) >= 1);
}

#[tokio::test]
async fn openai_runs_hosted_web_search() {
    let tools = OpenAiHostedTools {
        web_search: Some(WebSearchTool {
            search_context_size: Some(SearchContextSize::Low),
            ..Default::default()
        }),
        ..Default::default()
    };
    let prompt = "Search the web: what is the latest stable Rust release? Answer in one line.";
    assert_hosted_call(tools, prompt, "web_search", "web_search_call").await;
}

#[tokio::test]
async fn openai_runs_hosted_code_interpreter() {
    let tools = OpenAiHostedTools {
        code_interpreter: Some(ContainerTool::default()),
        ..Default::default()
    };
    let prompt = "Use python to compute 2**100. Reply with the number only.";
    assert_hosted_call(tools, prompt, "code_interpreter", "code_interpreter_call").await;
}

#[tokio::test]
async fn openai_runs_hosted_shell() {
    let tools = OpenAiHostedTools {
        shell: Some(ContainerTool::default()),
        ..Default::default()
    };
    let prompt = "Run `uname -s` in the shell and reply with its output only.";
    assert_hosted_call(tools, prompt, "shell", "shell_call").await;
}

#[tokio::test]
async fn openai_runs_hosted_file_search() {
    let store = VectorStore::create().await;
    let tools = OpenAiHostedTools {
        file_search: Some(FileSearchTool {
            vector_store_ids: vec![store.id.clone()],
            max_num_results: Some(3),
        }),
        ..Default::default()
    };
    let prompt = "Search the files: what is the code word in the document? Answer in one line.";
    let outcome = tokio::spawn(assert_hosted_call(
        tools,
        prompt,
        "file_search",
        "file_search_call",
    ))
    .await;
    // Delete the store and file even when the assertion failed.
    store.delete().await;
    if let Err(error) = outcome {
        std::panic::resume_unwind(error.into_panic());
    }
}

/// A vector store holding one small document, made for one run: the test needs
/// no store kept in the account between runs.
struct VectorStore {
    id: String,
    file_id: String,
    client: reqwest::Client,
    key: String,
}

impl VectorStore {
    const API: &str = "https://api.openai.com/v1";

    async fn create() -> Self {
        let key =
            std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set for the live test");
        let client = reqwest::Client::new();
        let form = reqwest::multipart::Form::new()
            .text("purpose", "assistants")
            .part(
                "file",
                reqwest::multipart::Part::text("The code word for this test is PERIWINKLE.\n")
                    .file_name("everruns-file-search-live.txt")
                    .mime_str("text/plain")
                    .unwrap(),
            );
        let file: serde_json::Value = client
            .post(format!("{}/files", Self::API))
            .bearer_auth(&key)
            .multipart(form)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .expect("upload the test document")
            .json()
            .await
            .unwrap();
        let file_id = file["id"].as_str().unwrap().to_string();
        let store: serde_json::Value = client
            .post(format!("{}/vector_stores", Self::API))
            .bearer_auth(&key)
            .json(&serde_json::json!({
                "name": "everruns-file-search-live",
                "file_ids": [file_id],
                "expires_after": {"anchor": "last_active_at", "days": 1},
            }))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .expect("create the vector store")
            .json()
            .await
            .unwrap();
        let store = Self {
            id: store["id"].as_str().unwrap().to_string(),
            file_id,
            client,
            key,
        };
        // Search finds nothing until the file is indexed.
        for _ in 0..60 {
            let status: serde_json::Value = store
                .client
                .get(format!("{}/vector_stores/{}", Self::API, store.id))
                .bearer_auth(&store.key)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            if status["file_counts"]["completed"].as_u64() == Some(1) {
                return store;
            }
            assert_eq!(
                status["file_counts"]["failed"].as_u64(),
                Some(0),
                "{status}"
            );
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        store.delete().await;
        panic!("the vector store did not index its file within a minute");
    }

    async fn delete(&self) {
        for path in [
            format!("vector_stores/{}", self.id),
            format!("files/{}", self.file_id),
        ] {
            let _ = self
                .client
                .delete(format!("{}/{path}", Self::API))
                .bearer_auth(&self.key)
                .send()
                .await;
        }
    }
}

/// Stream one request and return its text, tool calls, finish reason and
/// response id.
async fn run(
    config: &LlmCallConfig,
    messages: Vec<Message>,
) -> (String, Vec<ToolCall>, String, Option<String>) {
    let api_key =
        std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set for the live test");
    let mut stream = provider("openai", api_key)
        .chat_completion_stream(messages, config)
        .await
        .expect("OpenAI should accept the request");
    let (mut text, mut calls, mut finish, mut id) =
        (String::new(), Vec::new(), String::new(), None);
    while let Some(event) = stream.next().await {
        match event.expect("stream should not fail") {
            LlmStreamEvent::TextDelta(delta) => text.push_str(&delta),
            LlmStreamEvent::ToolCalls(tool_calls) => calls = tool_calls,
            LlmStreamEvent::Error(error) => panic!("stream error: {error:?}"),
            LlmStreamEvent::Done(meta) => {
                finish = meta.finish_reason.unwrap_or_default();
                id = meta.response_id;
            }
            _ => {}
        }
    }
    (text, calls, finish, id)
}

/// Remote MCP with approval: the first response stops at an approval request,
/// surfaced as a synthetic call; replaying the approval runs the MCP tool.
#[tokio::test]
async fn openai_remote_mcp_pauses_for_approval_and_resumes() {
    let mut config = LlmCallConfig::new(LIVE_MODEL);
    config.reasoning_effort = Some(ReasoningEffort::Low);
    let (key, value) = OpenAiHostedTools {
        mcp_servers: vec![McpServerTool {
            server_label: "deepwiki".into(),
            server_url: "https://mcp.deepwiki.com/mcp".into(),
            ..Default::default()
        }],
        ..Default::default()
    }
    .to_driver_option()
    .unwrap();
    config.driver_options.insert(key, value);

    let user = Message::text(
        MessageRole::User,
        "Use deepwiki to ask about repo facebook/react: what language is it written in? One line.",
    );
    let (_, calls, finish, response_id) = run(&config, vec![user.clone()]).await;
    eprintln!("approval calls: {calls:?}");
    assert_eq!(finish, "tool_calls");
    let approval = calls
        .iter()
        .find(|call| call.name == OPENAI_MCP_APPROVAL_TOOL)
        .expect("an approval request")
        .clone();

    // Stateless replay (approve, deny), then a `previous_response_id`
    // continuation, which sends only the approval response.
    for (answer, previous) in [
        (r#"{"approve":true}"#, None),
        ("denied", None),
        (r#"{"approve":true}"#, response_id),
    ] {
        let mut assistant = Message::text(MessageRole::Assistant, "");
        assistant.tool_calls = Some(vec![approval.clone()]);
        let mut result = Message::text(MessageRole::Tool, answer);
        result.tool_call_id = Some(approval.id.clone());
        let mut config = config.clone();
        config.previous_response_id = previous.clone();
        let messages = vec![user.clone(), assistant, result];
        let (text, calls, finish, _) = run(&config, messages).await;
        eprintln!(
            "{answer} stateful={}: {finish} {text} {calls:?}",
            previous.is_some()
        );
        assert!(
            !text.trim().is_empty() || !calls.is_empty(),
            "turn continues"
        );
    }
}
