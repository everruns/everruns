use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::event_usage;
use everruns::{Agent, CapabilityRef, InMemoryEngine, McpServer, Provider, Session, SessionEvent};
use mira::{ErrorKind, RunCx, Sample, Subject, Target, Transcript};
use serde_json::{Value, json};

const MAX_ITERATIONS: usize = 18;
const INSTRUCTIONS: &str = "You are an operations assistant for a synthetic company. Use the company's connected tools to answer the request. For state changes, identify the exact target, read its current state first, make only the requested change, then verify the result and preserve unrelated records. Do not ask for confirmation. Reply briefly.";

pub struct McpScaleSubject;
static RUN_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[async_trait::async_trait]
impl Subject for McpScaleSubject {
    async fn run(&self, sample: &Sample, cx: &RunCx) -> Transcript {
        let started = Instant::now();
        let provider = match provider(&cx.target) {
            Ok(provider) => provider,
            Err(error) => return Transcript::infra_error(error),
        };
        let Some(size) = cx
            .param("catalog_size")
            .and_then(|v| v.parse::<usize>().ok())
        else {
            return Transcript::infra_error("missing catalog_size axis");
        };
        let servers = cx
            .param("servers")
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(1);
        let surface = cx.param("surface").unwrap_or("native");
        let (session, state_paths) =
            match build_session(sample, &cx.target, provider, size, servers, surface) {
                Ok(session) => session,
                Err(error) => {
                    return Transcript::infra_error(format!("Framework build failed: {error}"));
                }
            };
        let mut transcript = run_turns(&session, sample).await;
        finish(&mut transcript, size, &state_paths);
        transcript.timing.duration_ms = started.elapsed().as_millis() as u64;
        for path in state_paths {
            let _ = std::fs::remove_file(&path);
            let mut lock_name = path.as_os_str().to_os_string();
            lock_name.push(".lock");
            let _ = std::fs::remove_file(std::path::PathBuf::from(lock_name));
        }
        transcript
    }
}

pub fn build_session(
    sample: &Sample,
    target: &Target,
    provider: Provider,
    size: usize,
    servers: usize,
    surface: &str,
) -> Result<(Session, Vec<std::path::PathBuf>), String> {
    if !["native", "cli", "hybrid"].contains(&surface) {
        return Err(format!("unknown surface: {surface}"));
    }
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let state_prefix = std::env::temp_dir().join(format!(
        "everruns-mcp-scale-{}-{}-{timestamp}-{sequence}",
        std::process::id(),
        sample.id
    ));
    let mut state_paths = Vec::new();
    let mut builder = Agent::builder()
        .name(format!("mcp-scale-{}-{surface}-{size}", sample.id))
        .instructions(INSTRUCTIONS)
        .provider(provider)
        .model(target.model.clone())
        .max_iterations(MAX_ITERATIONS);

    if surface != "cli" {
        builder = builder.capability(CapabilityRef::new("tool_search").config(json!({
            "bounded": true,
            "max_active_tools": 32,
            "max_active_bytes": 65536,
            "max_result_bytes": 32768
        })));
    }
    if surface != "native" {
        builder = builder.capability("bashkit_shell").capability(
            CapabilityRef::new("tools_in_shell").config(json!({
                "mode": surface,
                "rate_unlabeled_tools": false
            })),
        );
    }

    let server_count = servers.max(1);
    for index in 0..server_count {
        let per_server = server_catalog_size(size, server_count, index);
        let name = if index == 0 {
            "telemetry".to_string()
        } else {
            format!("catalog{index}")
        };
        let state_path = state_prefix.with_extension(format!("{name}.json"));
        state_paths.push(state_path.clone());
        let per_server_arg = per_server.to_string();
        let state_path_arg = state_path.to_string_lossy().into_owned();
        let mut server = McpServer::stdio(&name, executable.to_string_lossy()).args(
            [
                "--mcp-fixture",
                "--size",
                per_server_arg.as_str(),
                "--server-id",
                &name,
                "--page-size",
                "100",
                "--state-file",
                state_path_arg.as_str(),
            ]
            .map(str::to_owned),
        );
        if index == 0 {
            server = server.arg("--core");
        }
        if index == 0 {
            server =
                server.description("Geothermal drilling telemetry and field operations records");
        }
        builder = builder.mcp_server(server);
    }
    let agent = builder.build().map_err(|error| error.to_string())?;
    Ok((InMemoryEngine::new().create(agent), state_paths))
}

pub fn server_catalog_size(total: usize, servers: usize, index: usize) -> usize {
    if servers <= 1 {
        return total;
    }
    if index == 0 {
        return 8;
    }
    let peers = servers - 1;
    let remaining = total - 8;
    let base = remaining / peers;
    let extra = remaining % peers;
    let peer_index = index - 1;
    base + usize::from(peer_index < extra)
}

pub async fn run_turns(session: &Session, sample: &Sample) -> Transcript {
    let mut stream = session.events();
    let mut transcript = Transcript::default();
    for turn in &sample.input {
        match session.run(turn.clone()).await {
            Ok(result) => {
                transcript.final_response = result.response;
                transcript.iterations += result.iterations;
                if !result.success {
                    let error = result.error.unwrap_or_else(|| "turn failed".into());
                    transcript.error_kind = classify(&error);
                    transcript.error = Some(error);
                    break;
                }
            }
            Err(error) => {
                let error = error.to_string();
                transcript.error_kind = classify(&error);
                transcript.error = Some(error);
                break;
            }
        }
    }
    loop {
        match stream.try_recv() {
            Ok(Some(event)) => transcript.events.push(event_value(event)),
            Ok(None) => break,
            Err(error) => {
                transcript.error_kind = ErrorKind::Infra;
                transcript.error = Some(format!("Framework event stream incomplete: {error}"));
                break;
            }
        }
    }
    transcript
}

fn finish(transcript: &mut Transcript, catalog_size: usize, state_paths: &[std::path::PathBuf]) {
    let snapshots = state_paths
        .iter()
        .filter_map(|path| {
            std::fs::read(path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        })
        .collect::<Vec<_>>();
    transcript
        .metadata
        .insert("fixture_state_snapshots".into(), json!(snapshots));
    event_usage::summarize_events(&transcript.events).apply_to(transcript);
    transcript.tool_calls = transcript
        .events
        .iter()
        .filter(|event| event["type"] == "tool.completed")
        .filter_map(|event| {
            event
                .pointer("/data/tool_name")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    transcript.tool_calls_count = transcript.tool_calls.len();
    let mcp_calls: Vec<_> = transcript
        .events
        .iter()
        .filter(|event| {
            (event["type"] == "tool.completed"
                && event
                    .pointer("/data/tool_name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| name.starts_with("mcp_")))
                || (event["type"] == "tool.nested_call"
                    && event
                        .pointer("/data/tool_name")
                        .and_then(Value::as_str)
                        .is_some_and(|name| name.starts_with("mcp_"))
                    && event.pointer("/data/status").and_then(Value::as_str) == Some("completed"))
        })
        .filter_map(|event| event.pointer("/data/tool_name").and_then(Value::as_str))
        .map(str::to_owned)
        .collect();
    let unique_operations: std::collections::HashSet<_> = mcp_calls.iter().collect();
    transcript.record_metric("mcp.calls", mcp_calls.len() as f64);
    transcript.record_metric("mcp.unique_operations", unique_operations.len() as f64);
    transcript.record_metric("mcp.catalog_tools", catalog_size as f64);
    transcript.record_metric("mcp.input_tokens", transcript.usage.input_tokens as f64);
}

fn event_value(event: SessionEvent) -> Value {
    json!({"type":event.event_type(), "data":event.canonical_json().get("data").cloned().unwrap_or(Value::Null)})
}

pub fn provider(target: &Target) -> Result<Provider, String> {
    let key = |name: &str| std::env::var(name).map_err(|_| format!("missing API key: {name}"));
    Ok(match target.provider.as_str() {
        "anthropic" => {
            everruns_drivers::anthropic::provider("anthropic", key("ANTHROPIC_API_KEY")?)
        }
        "openai" => everruns_drivers::openai::provider("openai", key("OPENAI_API_KEY")?),
        "openrouter" => {
            everruns_drivers::openrouter::provider("openrouter", key("OPENROUTER_API_KEY")?)
        }
        other => return Err(format!("unsupported provider '{other}'")),
    })
}

fn classify(message: &str) -> ErrorKind {
    if mira::is_rate_limited(message) {
        return ErrorKind::Infra;
    }
    let lower = message.to_ascii_lowercase();
    // Provider-side model availability is infrastructure. Keep this narrow:
    // a missing catalog tool or operation is a subject failure.
    if lower.contains("model")
        && ["not available", "invalid", "unknown", "not found"]
            .iter()
            .any(|part| lower.contains(part))
    {
        return ErrorKind::Infra;
    }
    if [
        "402",
        "429",
        "500",
        "502",
        "503",
        "quota",
        "credit",
        "timed out",
        "timeout",
        "connection",
        "missing api key",
    ]
    .iter()
    .any(|part| lower.contains(part))
    {
        ErrorKind::Infra
    } else {
        ErrorKind::Subject
    }
}

#[cfg(test)]
mod classifier_tests {
    use super::classify;
    use mira::ErrorKind;

    #[test]
    fn model_provider_availability_is_infrastructure() {
        assert_eq!(
            classify("The model meta/muse-spark-1.3-contributor is not available"),
            ErrorKind::Infra
        );
    }

    #[test]
    fn missing_catalog_operation_is_a_subject_failure() {
        assert_eq!(
            classify("tool catalog operation is not available"),
            ErrorKind::Subject
        );
    }
}
