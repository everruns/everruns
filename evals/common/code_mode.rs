//! Shared surface attribution. Native schemas stay visible in Monty; a direct
//! tool fallback must not count as evidence that Python completed the task.
use everruns::AgentBuilder;
use mira::{Sample, Score, Scorer, Transcript};
use serde_json::Value;

pub fn configure(builder: AgentBuilder, surface: &str) -> Result<AgentBuilder, String> {
    Ok(match surface {
        "native" => builder,
        "cli" | "hybrid" => builder.capability("bashkit_shell").capability(
            everruns::CapabilityRef::new("shell_code_mode").config(serde_json::json!({
                "mode": surface, "rate_unlabeled_tools": false
            })),
        ),
        "monty" => builder.capability(everruns::PythonCodeMode),
        _ => return Err(format!("unknown surface: {surface}")),
    })
}

pub fn instruction(surface: &str) -> &'static str {
    match surface {
        "monty" => {
            "Use Python Code Mode for task operations. Discover deferred tools using native discovery when necessary, then use tool(name, arguments) inside python. Check the error field. After an approval decision, resume with the saved token; do not rerun source or call the pending operation directly."
        }
        "cli" => "Use the shell tools command for task operations.",
        "hybrid" => "Use either native tools or the shell tools command for task operations.",
        _ => "Use native tools for task operations.",
    }
}

/// Fail infrastructure qualification before spending model tokens. The operator
/// selects the worker; this never falls back to an unhardened binary.
pub async fn preflight(surface: &str) -> Result<(), String> {
    if surface != "monty" {
        return Ok(());
    }
    let directory = std::env::var_os("EVERRUNS_MONTY_STATE_DIR")
        .ok_or("Monty eval requires EVERRUNS_MONTY_STATE_DIR (private host storage)")?;
    let path = std::path::Path::new(&directory);
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !path.is_absolute() || !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("Monty journal must be an absolute private directory".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("Monty journal must have mode 0700".into());
        }
    }
    use everruns_contracts::runtime::tools::{Tool, ToolExecutionResult};
    use everruns_integrations::monty::{MontyConfig, PythonTool};
    let tool = PythonTool::new(MontyConfig {
        state_directory: None,
        ..Default::default()
    });
    let context = everruns_contracts::runtime::tool_context::ToolContext::new(
        everruns_contracts::typed_id::SessionId::new(),
    );
    match tool.execute_with_context(serde_json::json!({"script":"1 + 1"}), &context).await {
        ToolExecutionResult::Success(value) if value["result"] == 2 => Ok(()),
        _ => Err("Monty worker preflight failed; qualify the configured launcher before running model evals".into()),
    }
}

pub fn finish(transcript: &mut Transcript, surface: &str) {
    transcript.metadata.insert("surface".into(), surface.into());
    transcript
        .metadata
        .insert("placement".into(), "framework".into());
    if surface == "monty" {
        transcript.metadata.insert(
            "isolation".into(),
            std::env::var("EVERRUNS_EVAL_ISOLATION_LABEL")
                .unwrap_or_else(|_| "operator-configured; not independently qualified".into())
                .into(),
        );
    }
    let mut scripts = 0;
    let mut resumes = 0;
    let mut failed = 0;
    for event in &transcript.events {
        if event["type"] == "tool.started" && event["data"]["tool_call"]["name"] == "python" {
            let args = &event["data"]["tool_call"]["arguments"];
            scripts += usize::from(args.get("script").is_some());
            resumes += usize::from(args.get("resume").is_some());
        }
        if event["type"] == "tool.nested_call"
            && matches!(event["data"]["status"].as_str(), Some("refused" | "failed"))
        {
            failed += 1;
        }
    }
    transcript.record_metric("code.python_scripts", scripts as f64);
    transcript.record_metric("code.python_resumes", resumes as f64);
    transcript.record_metric("code.nested_failures", failed as f64);
}

fn task_tool(name: &str) -> bool {
    (name.starts_with("mcp_")
        && name
            .split_once("__")
            .is_some_and(|(_, operation)| !operation.is_empty()))
        || matches!(
            name,
            "weather_forecast"
                | "currency_convert"
                | "reserve_change"
                | "apply_change"
                | "read_change"
        )
}

pub fn surface_used() -> Box<dyn Scorer> {
    mira::scorer::scorer("surface_used", |_: &Sample, t: &Transcript| {
        let surface = t
            .metadata
            .get("surface")
            .and_then(Value::as_str)
            .unwrap_or("native");
        let expected = match surface {
            "monty" => "python",
            "cli" => "bash",
            _ => return Score::pass("surface_used", "native operations permitted"),
        };
        let parents: std::collections::HashSet<&str> = t
            .events
            .iter()
            .filter(|e| e["type"] == "tool.started" && e["data"]["tool_call"]["name"] == expected)
            .filter_map(|e| e["data"]["tool_call"]["id"].as_str())
            .collect();
        let direct = t.events.iter().any(|e| {
            e["type"] == "tool.started"
                && e["data"]["tool_call"]["name"]
                    .as_str()
                    .is_some_and(task_tool)
        });
        let nested = t.events.iter().any(|e| {
            e["type"] == "tool.nested_call"
                && e["data"]["status"] == "completed"
                && e["data"]["tool_name"].as_str().is_some_and(task_tool)
                && e["data"]["parent_tool_call_id"]
                    .as_str()
                    .is_some_and(|id| parents.contains(id))
        });
        if nested && !direct {
            Score::pass(
                "surface_used",
                format!("task operations ran through {expected}"),
            )
        } else {
            Score::fail(
                "surface_used",
                format!(
                    "expected {expected} task evidence; nested={nested}, direct_fallback={direct}"
                ),
            )
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[tokio::test]
    async fn direct_fallback_or_unrelated_parent_cannot_pass_monty() {
        let sample: Sample = serde_json::from_value(json!({"id":"test","input":["act"]})).unwrap();
        let mut t = Transcript::default();
        t.metadata.insert("surface".into(), "monty".into());
        t.events = vec![
            json!({"type":"tool.started","data":{"tool_call":{"id":"p1","name":"python"}}}),
            json!({"type":"tool.nested_call","data":{"parent_tool_call_id":"wrong","tool_name":"mcp_fixture__read","status":"completed"}}),
        ];
        assert!(!surface_used().score(&sample, &t).await.pass);
        t.events[1]["data"]["parent_tool_call_id"] = json!("p1");
        assert!(surface_used().score(&sample, &t).await.pass);
        t.events
            .push(json!({"type":"tool.started","data":{"tool_call":{"name":"mcp_fixture__"}}}));
        assert!(
            surface_used().score(&sample, &t).await.pass,
            "deferred discovery is permitted"
        );
        t.events
            .push(json!({"type":"tool.started","data":{"tool_call":{"name":"mcp_fixture__read"}}}));
        assert!(!surface_used().score(&sample, &t).await.pass);
    }
}
