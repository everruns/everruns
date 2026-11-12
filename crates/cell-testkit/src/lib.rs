//! Simulated models and test tools for the chaos and load runs, and the
//! agent definitions the test clients send.
//!
//! The split mirrors production. A node binary is a *host*: it brings model
//! providers and the code behind tools ([`host`]). Agents are data a client
//! sends with each session ([`definition`]); nodes know none of them.
//! Real-model behavior is shown by `examples/assistant`; the simulated
//! models here exist only so failure and scale tests are deterministic and
//! free.

pub mod fake_cp;
pub mod signer;

use std::io::Write;
use std::time::Duration;

use everruns::prelude::*;
use everruns::{LlmSimConfig, OnExhausted, SimToolCall, SimTurn};
use everruns_cell::StandardHost;
use serde_json::{Value, json};

fn env_ms(name: &str, default: u64) -> Duration {
    Duration::from_millis(
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default),
    )
}

/// Append `session` to the ledger file each time the `work` tool really runs,
/// so a check can compare real executions with `tool.started` events.
fn record(session: &str) {
    if let Ok(path) = std::env::var("CELL_LEDGER")
        && let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
    {
        let _ = file.write_all(format!("{session}\n").as_bytes());
    }
}

fn work_tool() -> FunctionTool {
    FunctionTool::with_context(
        "work",
        "Do one unit of work.",
        json!({ "type": "object", "properties": {} }),
        |context, _args: Value| async move {
            record(&context.session_id().to_string());
            tokio::time::sleep(env_ms("CELL_WORK_MS", 50)).await;
            Ok::<_, String>(json!({ "done": true }))
        },
    )
}

/// A tool a person must approve before it runs (the approval gate).
fn guarded_tool() -> FunctionTool {
    FunctionTool::new(
        "guarded",
        "Change something that needs a person's approval.",
        json!({ "type": "object", "properties": {} }),
        |_args: Value| async move { Ok::<_, String>(json!({ "changed": true })) },
    )
    .always_needs_approval()
}

/// A tool that changes something and says nothing about its risk: the
/// `tool_approval` capability in its protective mode gates it.
fn note_tool() -> FunctionTool {
    FunctionTool::new(
        "note",
        "Write a note.",
        json!({ "type": "object", "properties": {} }),
        |_args: Value| async move { Ok::<_, String>(json!({ "noted": true })) },
    )
}

/// A simulated model that calls `tool`, calls it again once told it was
/// approved (as the Engine asks), then answers.
fn approve_model(tool: &str) -> Model {
    let call = || {
        SimTurn::ToolCalls(vec![SimToolCall {
            name: tool.into(),
            arguments: json!({}),
            id: None,
        }])
    };
    let sim = LlmSimConfig::scripted(vec![
        call(),
        call(),
        SimTurn::Assistant("Changed, as approved.".into()),
    ])
    .with_on_exhausted(OnExhausted::Loop);
    Model::simulated_with_config(sim)
}

fn work_call() -> SimTurn {
    SimTurn::ToolCalls(vec![SimToolCall {
        name: "work".into(),
        arguments: json!({}),
        id: None,
    }])
}

/// A simulated model that calls `work` `steps` times, then answers.
fn worker_model(steps: usize, delay: Option<Duration>) -> Model {
    let mut script: Vec<SimTurn> = (0..steps).map(|_| work_call()).collect();
    script.push(SimTurn::Assistant("All work done.".into()));
    let mut sim = LlmSimConfig::scripted(script).with_on_exhausted(OnExhausted::Loop);
    if let Some(delay) = delay {
        sim = sim.with_response_delay(delay);
    }
    Model::simulated_with_config(sim)
}

/// llmsim with native compaction, the way OpenAI's Responses API compacts:
/// `compact` replaces the history with one opaque item. The engine installs
/// that as a durable checkpoint and later turns send it in place of the
/// messages it covers, so long chats exercise checkpoints end to end.
struct CompactingSim(everruns_llmsim::LlmSimDriver);

#[async_trait::async_trait]
impl everruns::ChatDriver for CompactingSim {
    async fn chat_completion_stream(
        &self,
        endpoint: &everruns::ProviderEndpoint,
        messages: Vec<everruns_contracts::Message>,
        config: &everruns::LlmCallConfig,
    ) -> everruns_contracts::Result<everruns::LlmResponseStream> {
        self.0
            .chat_completion_stream(endpoint, messages, config)
            .await
    }

    fn supports_compact(&self) -> bool {
        true
    }

    async fn compact(
        &self,
        _endpoint: &everruns::ProviderEndpoint,
        request: everruns_contracts::CompactRequest,
    ) -> everruns_contracts::Result<Option<everruns_contracts::CompactResponse>> {
        Ok(Some(everruns_contracts::CompactResponse {
            output: vec![everruns_contracts::CompactOutputItem::Compaction {
                encrypted_content: format!("compacted {} items", request.input.len()),
            }],
            usage: None,
        }))
    }
}

/// A chat model: about a hundred tokens an answer, so a session's context
/// fills (and is compacted) within a few hundred turns.
fn chat_model() -> Model {
    let answer = "Noted; here is what that means for the plan. ".repeat(8);
    Model::new(
        "chat",
        everruns::Provider::new(
            "llmsim",
            CompactingSim(everruns_llmsim::LlmSimDriver::new(LlmSimConfig::fixed(
                answer,
            ))),
        ),
    )
}

/// A model over the session's own workspace: `write <path> <text>` writes a
/// file, `read <path>` reads one, and the answer after the tool quotes the
/// tool's result. What it plays depends only on the conversation, so any
/// executor, before or after a move, plays the same step.
struct FilesSim;

#[async_trait::async_trait]
impl everruns::ChatDriver for FilesSim {
    async fn chat_completion_stream(
        &self,
        endpoint: &everruns::ProviderEndpoint,
        messages: Vec<everruns_contracts::Message>,
        config: &everruns::LlmCallConfig,
    ) -> everruns_contracts::Result<everruns::LlmResponseStream> {
        use everruns_contracts::MessageRole;
        let turn = match messages.last() {
            Some(last) if last.role == MessageRole::Tool => {
                SimTurn::Assistant(format!("Result: {}", last.content.to_text()))
            }
            _ => {
                let text = messages
                    .iter()
                    .rev()
                    .find(|m| m.role == MessageRole::User)
                    .map(|m| m.content.to_text())
                    .unwrap_or_default();
                let mut words = text.trim().splitn(3, ' ');
                let call = |name: &str, arguments: Value| {
                    SimTurn::ToolCalls(vec![SimToolCall {
                        name: name.into(),
                        arguments,
                        id: None,
                    }])
                };
                match (words.next(), words.next(), words.next()) {
                    (Some("write"), Some(path), content) => call(
                        "write_file",
                        json!({ "path": path, "content": content.unwrap_or_default() }),
                    ),
                    (Some("read"), Some(path), _) => call("read_file", json!({ "path": path })),
                    _ => SimTurn::Assistant("Say write or read.".into()),
                }
            }
        };
        everruns_llmsim::LlmSimDriver::new(LlmSimConfig::scripted(vec![turn]))
            .chat_completion_stream(endpoint, messages, config)
            .await
    }
}

/// Deltas a `stamp` turn streams.
pub const STAMPS: usize = 20;
/// Time between a `stamp` turn's deltas.
pub const STAMP_EVERY: std::time::Duration = std::time::Duration::from_millis(50);

/// A model whose deltas carry the time they were produced.
struct StampSim;

#[async_trait::async_trait]
impl everruns::ChatDriver for StampSim {
    async fn chat_completion_stream(
        &self,
        _endpoint: &everruns::ProviderEndpoint,
        _messages: Vec<everruns_contracts::Message>,
        _config: &everruns::LlmCallConfig,
    ) -> everruns_contracts::Result<everruns::LlmResponseStream> {
        use everruns_contracts::driver_registry::{LlmCompletionMetadata, LlmStreamEvent};
        let stream = futures::stream::unfold(0usize, |n| async move {
            if n > STAMPS {
                return None;
            }
            if n == STAMPS {
                let mut done = LlmCompletionMetadata::default();
                done.prompt_tokens = Some(1);
                done.completion_tokens = Some(STAMPS as u32);
                done.total_tokens = Some(STAMPS as u32 + 1);
                return Some((Ok(LlmStreamEvent::Done(Box::new(done))), n + 1));
            }
            if n > 0 {
                tokio::time::sleep(STAMP_EVERY).await;
            }
            let micros = chrono::Utc::now().timestamp_micros();
            Some((Ok(LlmStreamEvent::TextDelta(format!("@{micros};"))), n + 1))
        });
        Ok(Box::pin(stream))
    }
}

fn stamp_model() -> Model {
    Model::new("stamp", everruns::Provider::new("llmsim", StampSim))
}

fn files_model() -> Model {
    Model::new("files", everruns::Provider::new("llmsim", FilesSim))
}

/// A simulated model that hands the message to the member named `worker3`
/// through `delegate`, then reports.
fn lead_model() -> Model {
    let sim = LlmSimConfig::scripted(vec![
        SimTurn::ToolCalls(vec![SimToolCall {
            name: "delegate".into(),
            arguments: json!({ "agent": "worker3", "text": "Do the work." }),
            id: None,
        }]),
        SimTurn::Assistant("The member finished.".into()),
    ])
    .with_on_exhausted(OnExhausted::Loop);
    Model::simulated_with_config(sim)
}

/// Hand work to another agent's session and return its answer.
///
/// The member is one of the agents bundled in the calling session's own
/// definition (`agents`, by name), so the client that defined the lead also
/// defined its members. Hand-off goes over the runtime's own API: the child
/// session and its input get ids derived from this call's idempotency key,
/// so the tool is idempotent. If the lead's executor dies mid-call, resume
/// runs it again, and the retry finds the same child (and its turn) instead
/// of starting a second one.
fn delegate_tool() -> FunctionTool {
    FunctionTool::with_context(
        "delegate",
        "Ask another agent to do a task and wait for its answer.",
        json!({
            "type": "object",
            "properties": { "agent": { "type": "string" }, "text": { "type": "string" } },
            "required": ["agent", "text"]
        }),
        |context, args: Value| async move {
            let key = context
                .idempotency_key()
                .map(str::to_string)
                .unwrap_or_else(|| context.tool_call_id().to_string());
            let derive = |kind: &str| {
                let uuid = uuid::Uuid::new_v5(
                    &uuid::Uuid::NAMESPACE_OID,
                    format!("{kind}:{key}").as_bytes(),
                );
                format!("{kind}_{}", uuid.simple())
            };
            let child = derive("session");
            let base =
                std::env::var("CELL_GATEWAY").map_err(|_| "CELL_GATEWAY not set".to_string())?;
            let http = reqwest::Client::builder()
                .no_proxy()
                .build()
                .map_err(|e| e.to_string())?;
            let post = |path: String, body: Value| {
                let http = http.clone();
                let url = format!("http://{base}{path}");
                async move {
                    let response = http
                        .post(url)
                        .json(&body)
                        .send()
                        .await
                        .map_err(|e| e.to_string())?;
                    if !response.status().is_success() {
                        return Err(format!(
                            "{}: {}",
                            response.status(),
                            response.text().await.unwrap_or_default()
                        ));
                    }
                    response.json::<Value>().await.map_err(|e| e.to_string())
                }
            };
            let parent = context.session_id().to_string();
            let definition: Value = http
                .get(format!("http://{base}/v1/sessions/{parent}/agent"))
                .send()
                .await
                .map_err(|e| e.to_string())?
                .json()
                .await
                .map_err(|e| e.to_string())?;
            let name = args["agent"].as_str().unwrap_or_default();
            let member = definition["agents"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|agent| agent["name"] == name)
                .cloned()
                .ok_or_else(|| format!("this agent has no member {name}"))?;
            post(
                "/v1/sessions".into(),
                json!({ "id": child, "agent": member }),
            )
            .await?;
            post(
                format!("/v1/sessions/{child}/messages"),
                json!({ "id": derive("message"), "text": args["text"] }),
            )
            .await?;
            loop {
                let view: Value = http
                    .get(format!(
                        "http://{base}/v1/sessions/{child}/wait?turns=1&timeout_ms=30000"
                    ))
                    .send()
                    .await
                    .map_err(|e| e.to_string())?
                    .json()
                    .await
                    .map_err(|e| e.to_string())?;
                if view["turns_completed"].as_u64().unwrap_or(0) >= 1 {
                    break;
                }
            }
            let events: Value = http
                .get(format!("http://{base}/v1/sessions/{child}/events"))
                .send()
                .await
                .map_err(|e| e.to_string())?
                .json()
                .await
                .map_err(|e| e.to_string())?;
            let answer = events["data"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|e| e["type"] == "output.message.completed")
                .filter_map(|e| e["data"]["message"]["content"].as_array())
                .flatten()
                .filter_map(|part| part["text"].as_str())
                .next_back()
                .unwrap_or_default()
                .to_string();
            Ok::<_, String>(json!({ "session": child, "answer": answer }))
        },
    )
    .idempotent()
}

// ------------------------------------------------------------ scenario agent

/// The research notes the Everruns llmsim scenarios read
/// (`/load-test/research.md`).
const RESEARCH: &str =
    "# Research notes\n\nCells hold sessions in memory and replicate them with Raft.\n";

fn scale() -> f64 {
    std::env::var("CELL_TOOL_SCALE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0)
}

/// A read-only file tool over a fixed `/load-test` tree.
fn file_tool(name: &'static str, description: &'static str) -> FunctionTool {
    FunctionTool::with_context(
        name,
        description,
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "offset": { "type": "integer" },
                "limit": { "type": "integer" }
            },
            "required": ["path"]
        }),
        move |context, args: Value| async move {
            record(&context.session_id().to_string());
            tokio::time::sleep(env_ms("CELL_FILE_MS", 5).mul_f64(scale())).await;
            let path = args["path"].as_str().unwrap_or("/load-test");
            Ok::<_, String>(match name {
                "list_directory" => json!({ "path": path, "entries": ["research.md"] }),
                "stat_file" => json!({ "path": path, "size": RESEARCH.len(), "kind": "file" }),
                _ => json!({ "path": path, "content": RESEARCH }),
            })
        },
    )
    .idempotent()
}

/// `bash` for the long-tool scenario: honours `sleep N`, scaled by
/// `CELL_TOOL_SCALE`, and echoes the rest. Not idempotent, so a cut-off
/// call is reported as interrupted rather than run again.
fn bash_tool() -> FunctionTool {
    FunctionTool::with_context(
        "bash",
        "Run shell commands.",
        json!({
            "type": "object",
            "properties": { "commands": { "type": "string" }, "timeout_ms": { "type": "integer" } },
            "required": ["commands"]
        }),
        |context, args: Value| async move {
            record(&context.session_id().to_string());
            let commands = args["commands"].as_str().unwrap_or_default().to_string();
            let mut output = Vec::new();
            for part in commands.split(';').map(str::trim) {
                if let Some(secs) = part
                    .strip_prefix("sleep ")
                    .and_then(|s| s.parse::<f64>().ok())
                {
                    tokio::time::sleep(Duration::from_secs_f64(secs * scale())).await;
                } else if let Some(text) = part.strip_prefix("echo ") {
                    output.push(text.to_string());
                }
            }
            Ok::<_, String>(json!({ "exit_code": 0, "stdout": output.join("\n") }))
        },
    )
}

/// `blob`: returns `kb` kilobytes of text, for tests of large tool results.
fn blob_tool() -> FunctionTool {
    FunctionTool::with_context(
        "blob",
        "Return a large text.",
        json!({
            "type": "object",
            "properties": { "kb": { "type": "integer" } },
            "required": ["kb"]
        }),
        |_context, args: Value| async move {
            let kb = args["kb"].as_u64().unwrap_or(1) as usize;
            let line = "The quick brown fox jumps over the lazy dog, again and again. ";
            let text: String = line.repeat(kb * 1024 / line.len() + 1);
            Ok::<_, String>(json!({ "text": &text[..kb * 1024] }))
        },
    )
    .idempotent()
}

/// A model that calls `blob` with the size (in KB) given in the message
/// (`blob <kb>`), then answers.
fn blob_model() -> Model {
    Model::new(
        "blob",
        everruns::Provider::new(
            "llmsim",
            CompactingSim(everruns_llmsim::LlmSimDriver::new(BLOB_SIM.clone())),
        ),
    )
}

static BLOB_SIM: std::sync::LazyLock<LlmSimConfig> = std::sync::LazyLock::new(|| {
    let kb: u64 = std::env::var("CELL_BLOB_KB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(64);
    LlmSimConfig::scripted(vec![
        SimTurn::ToolCalls(vec![SimToolCall {
            name: "blob".into(),
            arguments: json!({ "kb": kb }),
            id: None,
        }]),
        SimTurn::Assistant("Read it.".into()),
    ])
    .with_on_exhausted(OnExhausted::Loop)
});

/// The test node's host: simulated models and the test tools.
///
/// Models (`model.provider` = `llmsim`):
/// - `work-1`, `work-3`: call `work` that many times, then answer;
/// - `slow`: `work-1` behind a slow model call (`CELL_MODEL_DELAY_MS`);
/// - `lead`: delegate to the member `worker3`, then report;
/// - `chat`: a long answer, with no tools, and native compaction;
/// - `files`: writes or reads a workspace file the message names;
/// - `slow-chat`: a plain answer after `CELL_MODEL_DELAY_MS` (3 s);
/// - `approve`: calls `guarded`, which waits for a person's approval,
///   then answers;
/// - `gate`: the same with `note`, which the agent's `tool_approval`
///   capability gates;
/// - `stamp`: streams [`STAMPS`] text deltas, each the Unix time in
///   microseconds it was produced (`@<micros>;`), [`STAMP_EVERY`] apart:
///   the streaming latency bench reads them off the wire;
/// - anything else: plays the Everruns llmsim scenario named by a
///   `[[llmsim:<scenario>]]` marker in the message, and otherwise answers
///   "done.".
///   Scenario steps depend only on the conversation, so a turn resumed on
///   another executor plays the same step.
pub fn host() -> StandardHost {
    StandardHost::new()
        .provider("llmsim", |model| {
            Some(match model {
                "work-1" => worker_model(1, None),
                "work-3" => worker_model(3, None),
                "slow" => worker_model(1, Some(env_ms("CELL_MODEL_DELAY_MS", 3000))),
                "lead" => lead_model(),
                "chat" => chat_model(),
                "blob" => blob_model(),
                "files" => files_model(),
                "stamp" => stamp_model(),
                "approve" => approve_model("guarded"),
                "gate" => approve_model("note"),
                // A plain answer that takes `CELL_MODEL_DELAY_MS`: a turn
                // to cancel, for an agent the control plane defines.
                "slow-chat" => Model::simulated_with_config(
                    LlmSimConfig::fixed("Thought about it for a while.")
                        .with_response_delay(env_ms("CELL_MODEL_DELAY_MS", 3000)),
                ),
                _ => Model::simulated("done."),
            })
        })
        .tool(work_tool)
        .tool(guarded_tool)
        .tool(note_tool)
        .tool(delegate_tool)
        .tool(|| file_tool("list_directory", "List a directory."))
        .tool(|| file_tool("read_file", "Read a file."))
        .tool(|| file_tool("stat_file", "Describe a file."))
        .tool(bash_tool)
        .tool(blob_tool)
}

// ------------------------------------------------------------ client side

/// The `tools` entries for `names`, from the host's own schemas (a real
/// client would read them from `GET /v1/tools`).
#[expect(
    clippy::expect_used,
    reason = "the names are this file's own test tools; a missing one is a typo the first run must stop on"
)]
fn tools(names: &[&str]) -> Vec<Value> {
    let all = host().tool_definitions();
    names
        .iter()
        .map(|name| {
            all.iter()
                .find(|tool| tool["name"] == *name)
                .cloned()
                .expect("a test tool")
        })
        .collect()
}

fn agent(name: &str, instructions: &str, model: &str, tool_names: &[&str]) -> Value {
    json!({
        "name": name,
        "instructions": instructions,
        "model": { "provider": "llmsim", "model": model },
        "tools": tools(tool_names),
    })
}

/// The agent definitions the test clients send, by name:
/// - `echo`: answers;
/// - `worker`, `worker3`: one or three `work` steps, then an answer;
/// - `slow`: one step behind a slow model call;
/// - `sim`: plays the llmsim scenario named in each message, with file and
///   shell tools;
/// - `chat`: answers, with context compaction (for long sessions);
/// - `files`: writes and reads its workspace (`write <path> <text>`,
///   `read <path>`);
/// - `lead`: hands each message to its bundled member `worker3`.
pub fn definition(name: &str) -> Option<Value> {
    Some(match name {
        "echo" => agent("echo", "Answer.", "echo", &[]),
        // Deltas stamped with the time they were made (the latency bench).
        "stamp" => agent("stamp", "Answer.", "stamp", &[]),
        // Calls a tool that waits for a person's approval.
        "approve" => agent("approve", "Ask, then change.", "approve", &["guarded"]),
        // Its `tool_approval` capability gates a state-changing tool.
        "gate" => {
            let mut gate = agent("gate", "Ask, then change.", "gate", &["note"]);
            gate["capabilities"] = json!([{
                "ref": "tool_approval",
                "config": { "mode": "protective" }
            }]);
            gate
        }
        "worker" => agent("worker", "Do the work, then report.", "work-1", &["work"]),
        "worker3" => agent("worker3", "Do the work, then report.", "work-3", &["work"]),
        "slow" => agent("slow", "Do the work, then report.", "slow", &["work"]),
        "sim" => agent(
            "sim",
            "Research with the file tools, then answer.",
            "scenarios",
            &["list_directory", "read_file", "stat_file", "bash"],
        ),
        "chat" => {
            // Many short turns that keep the context growing, so the engine
            // compacts it (a checkpoint) every few hundred turns.
            let mut chat = agent("chat", "Answer.", "chat", &[]);
            chat["capabilities"] = json!([{
                "ref": "compaction",
                "config": { "strategy": "native", "budget_percent": 0.1 }
            }]);
            chat
        }
        "blob" => {
            // A large tool result each turn (`CELL_BLOB_KB` on the node).
            let mut blob = agent("blob", "Read, then answer.", "blob", &["blob"]);
            blob["capabilities"] = json!([{
                "ref": "compaction",
                "config": { "strategy": "native", "budget_percent": 0.1 }
            }]);
            blob
        }
        "files" => {
            // The session's durable workspace, through the Framework's own
            // file tools.
            let mut files = agent("files", "Use the files.", "files", &[]);
            files["capabilities"] = json!([{ "ref": "session_file_system" }]);
            files
        }
        "lead" => {
            let mut lead = agent("lead", "Delegate, then report.", "lead", &["delegate"]);
            lead["agents"] = json!([definition("worker3")?]);
            lead
        }
        _ => return None,
    })
}
