//! Spawn five child agents, receive task IDs immediately, then wait for every
//! child to finish. The in-memory task table represents an application-owned
//! task registry; every child is an ordinary Everruns agent with its own
//! session and context.
//!
//! ```text
//! cargo run -p everruns --features openai --example subagents
//! ```

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use everruns::{Agent, Engine, FunctionTool, OpenAI, ToolResponse};
use serde_json::json;
use tokio::task::JoinHandle;

type ChildResult = Result<String, String>;
const MODEL_ID: &str = "gpt-5.6-terra";

fn validate_wait_task_ids(task_ids: &[String]) -> Result<(), &'static str> {
    if task_ids.len() != 5 {
        return Err("wait_for_subagents expects five task IDs");
    }
    if task_ids.iter().collect::<HashSet<_>>().len() != task_ids.len() {
        return Err("wait_for_subagents expects five unique task IDs");
    }
    Ok(())
}

#[derive(Default)]
struct ChildTasks {
    next_task: AtomicU64,
    running: Mutex<HashMap<String, JoinHandle<ChildResult>>>,
    completed: Mutex<Vec<(String, String)>>,
}

fn spawn_tool(tasks: Arc<ChildTasks>) -> FunctionTool {
    FunctionTool::new(
        "spawn_subagents",
        "Spawn exactly five independent child agents and return their task IDs immediately.",
        json!({
            "type": "object",
            "properties": {
                "tasks": {
                    "type": "array",
                    "minItems": 5,
                    "maxItems": 5,
                    "items": {"type": "string"}
                }
            },
            "required": ["tasks"],
            "additionalProperties": false
        }),
        move |arguments| {
            let tasks = tasks.clone();
            async move {
                let instructions = arguments
                    .get("tasks")
                    .and_then(|value| value.as_array())
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(|value| value.as_str().map(str::to_owned))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                if instructions.len() != 5 {
                    return Ok::<_, String>(ToolResponse::error(
                        "tasks must contain exactly five strings",
                    ));
                }

                let mut spawned = Vec::with_capacity(5);
                let mut handles = Vec::with_capacity(5);
                for (index, instruction) in instructions.into_iter().enumerate() {
                    let task_id = format!(
                        "subagent_{}",
                        tasks.next_task.fetch_add(1, Ordering::Relaxed) + 1
                    );
                    let handle = tokio::spawn(async move {
                        // Different durations make it visible that every child
                        // starts before the parent begins waiting.
                        tokio::time::sleep(Duration::from_millis(40 * (5 - index) as u64)).await;
                        let child = Agent::builder()
                            .name(format!("worker-{}", index + 1))
                            .instructions("Complete the assigned independent task.")
                            .provider(OpenAI::from_env().map_err(|error| error.to_string())?)
                            .model(MODEL_ID)
                            .build()
                            .map_err(|error| error.to_string())?;
                        let result = Engine::new()
                            .create(child)
                            .run(instruction)
                            .await
                            .map_err(|error| error.to_string())?;
                        Ok(result.response)
                    });
                    spawned.push(task_id.clone());
                    handles.push((task_id, handle));
                }

                let mut running = tasks
                    .running
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                for (task_id, handle) in handles {
                    running.insert(task_id, handle);
                }

                Ok(ToolResponse::json(json!({
                    "task_ids": spawned,
                    "state": "running"
                })))
            }
        },
    )
}

fn wait_tool(tasks: Arc<ChildTasks>) -> FunctionTool {
    FunctionTool::new(
        "wait_for_subagents",
        "Wait until all named subagent tasks finish and return every result.",
        json!({
            "type": "object",
            "properties": {
                "task_ids": {
                    "type": "array",
                    "minItems": 5,
                    "maxItems": 5,
                    "uniqueItems": true,
                    "items": {"type": "string"}
                }
            },
            "required": ["task_ids"],
            "additionalProperties": false
        }),
        move |arguments| {
            let tasks = tasks.clone();
            async move {
                let task_ids = arguments
                    .get("task_ids")
                    .and_then(|value| value.as_array())
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(|value| value.as_str().map(str::to_owned))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                if let Err(error) = validate_wait_task_ids(&task_ids) {
                    return Ok::<_, String>(ToolResponse::error(error));
                }

                let handles = {
                    let mut running = tasks
                        .running
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    if let Some(unknown) = task_ids
                        .iter()
                        .find(|task_id| !running.contains_key(*task_id))
                    {
                        return Ok(ToolResponse::error(format!(
                            "unknown or already-waited task: {unknown}"
                        )));
                    }
                    let mut handles = Vec::with_capacity(task_ids.len());
                    for task_id in &task_ids {
                        let Some(handle) = running.remove(task_id) else {
                            return Ok(ToolResponse::error(format!(
                                "unknown or already-waited task: {task_id}"
                            )));
                        };
                        handles.push((task_id.clone(), handle));
                    }
                    handles
                };

                let mut results = Vec::with_capacity(handles.len());
                for (task_id, handle) in handles {
                    let result = match handle.await {
                        Ok(Ok(response)) => response,
                        Ok(Err(error)) => format!("failed: {error}"),
                        Err(error) => format!("join failed: {error}"),
                    };
                    results.push((task_id, result));
                }
                *tasks
                    .completed
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = results.clone();

                Ok(ToolResponse::json(json!({
                    "state": "succeeded",
                    "results": results
                })))
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::validate_wait_task_ids;

    #[test]
    fn wait_task_ids_must_be_unique() {
        let task_ids = ["one", "two", "three", "four", "one"].map(str::to_owned);

        assert_eq!(
            validate_wait_task_ids(&task_ids),
            Err("wait_for_subagents expects five unique task IDs")
        );
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let child_tasks = Arc::new(ChildTasks::default());
    let task_descriptions = [
        "inspect the event protocol",
        "inspect cancellation behavior",
        "inspect tool concurrency",
        "inspect session replay",
        "inspect capability composition",
    ];
    let parent = Agent::builder()
        .name("coordinator")
        .instructions("Delegate five independent reviews, then wait for and summarize all results.")
        .provider(OpenAI::from_env()?)
        .model(MODEL_ID)
        .tool(spawn_tool(child_tasks.clone()))
        .tool(wait_tool(child_tasks.clone()))
        .build()?;

    let prompt = format!(
        "Delegate each of these five independent reviews, then wait for every task ID:\n- {}",
        task_descriptions.join("\n- ")
    );
    let result = Engine::new()
        .create(parent.clone())
        .send_and_wait(prompt)
        .await?;
    for (task_id, response) in child_tasks
        .completed
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .iter()
    {
        println!("{task_id}: {response}");
    }
    println!("agent: {}", result.response);

    Ok(())
}
