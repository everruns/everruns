//! Real-model dreaming example and small behavioral eval. No simulated model.
//! OPENAI_API_KEY=... cargo run -p everruns --features openai --example dreaming -- --eval
#[cfg(feature = "openrouter")]
use everruns::OpenRouter;
use everruns::{Agent, DreamDestination, DreamTriggerMode, DreamingConfig, Engine, OpenAI};
use serde::Deserialize;
#[derive(Deserialize)]
struct Case {
    name: String,
    messages: Vec<String>,
    required: Vec<String>,
    forbidden: Vec<String>,
    #[serde(default)]
    forbidden_memory: Vec<String>,
    #[serde(default)]
    question: Option<String>,
    #[serde(default)]
    consolidate_after: Vec<usize>,
}

// A directory listing is not recall evidence. Count only content returned from
// a learned artifact, including genuine grep matches, that contains the facts.
fn reads_learned_memory(
    name: &str,
    arguments: &serde_json::Value,
    result: &serde_json::Value,
    required: &[String],
) -> bool {
    let learned = |path: &str| path.starts_with("/memory/user/learned/");
    let contains_facts = |value: &serde_json::Value| {
        let text = value.to_string().to_lowercase();
        required
            .iter()
            .all(|fact| text.contains(&fact.to_lowercase()))
    };
    match name {
        "read_file" => {
            arguments
                .get("path")
                .and_then(|v| v.as_str())
                .is_some_and(learned)
                && contains_facts(result)
        }
        "read_many_files" => result
            .get("results")
            .and_then(|v| v.as_array())
            .is_some_and(|files| {
                files.iter().any(|file| {
                    file.get("path")
                        .and_then(|v| v.as_str())
                        .is_some_and(learned)
                        && file.get("error").is_none_or(serde_json::Value::is_null)
                        && file.get("content").is_some_and(contains_facts)
                })
            }),
        "grep" | "grep_files" => ["matches", "blocks"]
            .into_iter()
            .filter_map(|key| result.get(key).and_then(|v| v.as_array()))
            .flatten()
            .any(|entry| {
                entry
                    .get("path")
                    .and_then(|v| v.as_str())
                    .is_some_and(learned)
                    && entry
                        .get("line")
                        .or_else(|| entry.get("lines"))
                        .is_some_and(contains_facts)
            }),
        _ => false,
    }
}

fn memory_snapshot(
    root: &std::path::Path,
) -> std::io::Result<std::collections::BTreeMap<std::path::PathBuf, Option<Vec<u8>>>> {
    let mut snapshot = std::collections::BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                return Err(std::io::Error::other("unexpected symlink in eval Memory"));
            }
            let content = if kind.is_dir() {
                pending.push(path.clone());
                None
            } else {
                Some(std::fs::read(&path)?)
            };
            snapshot.insert(path, content);
        }
    }
    Ok(snapshot)
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let eval = std::env::args().any(|a| a == "--eval");
    let openrouter = std::env::args().any(|a| a == "--openrouter");
    let model = if openrouter {
        "openai/gpt-6.1-sol"
    } else {
        "gpt-6.1-sol"
    };
    let cases: Vec<Case> = if eval {
        serde_json::from_str(include_str!("data/dreaming-cases.json"))?
    } else {
        vec![Case {
            name: "example".into(),
            messages: vec![
                "For future tasks, remember I prefer concise Rust examples and UTC timestamps."
                    .into(),
            ],
            required: vec!["Rust".into(), "UTC".into()],
            forbidden: vec![],
            forbidden_memory: vec![],
            question: Some(
                "What are my saved preferences? Read the learned Memory files before answering."
                    .into(),
            ),
            consolidate_after: vec![],
        }]
    };
    let root = std::env::current_dir()?.join("work/dreaming-example");
    std::fs::create_dir_all(&root)?;
    let mut passed = 0;
    for case in &cases {
        let directory = root.join(format!("{}-{}", case.name, uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory)?;
        let mut config = DreamingConfig::new(DreamDestination {
            mount: "/memory/user".into(),
            path: "/learned".into(),
            instructions:
                "Remember useful explicit preferences and verified lessons. Preserve corrections."
                    .into(),
        });
        config.trigger.mode = DreamTriggerMode::Manual;
        let builder = Agent::builder().name("dreaming-example").instructions(
            "Acknowledge new information briefly without writing any files. When asked to recall saved information, read existing learned Memory files and answer from them.",
        );
        let builder = if openrouter {
            #[cfg(feature = "openrouter")]
            {
                builder.provider(OpenRouter::from_env()?)
            }
            #[cfg(not(feature = "openrouter"))]
            {
                return Err("--openrouter requires the openrouter Cargo feature".into());
            }
        } else {
            builder.provider(OpenAI::from_env()?)
        };
        let agent = builder
            .model(model)
            .memory_directory("/memory/user", &directory)
            .capability(config)
            .build()?;
        let engine = Engine::new();
        let session = engine.create(agent.clone());
        for (index, message) in case.messages.iter().enumerate() {
            let before = memory_snapshot(&directory)?;
            session.send_and_wait(message.as_str()).await?;
            if memory_snapshot(&directory)? != before {
                return Err("foreground changed Memory before consolidation".into());
            }
            if case.consolidate_after.contains(&(index + 1))
                && !engine.dream(session.session_id()).await?
            {
                return Err("intermediate dreaming did not complete".into());
            }
        }
        if !engine.dream(session.session_id()).await? {
            return Err("dreaming did not complete".into());
        }
        let mut content = String::new();
        let learned = directory.join("learned");
        if learned.exists() {
            for file in std::fs::read_dir(&learned)? {
                content.push_str(&std::fs::read_to_string(file?.path())?);
            }
        }
        let recall = engine.create(agent);
        let question = case.question.as_deref().unwrap_or(
            "Read the learned Memory files and summarize the saved information, including current facts, preferences, outcomes, and procedures."
        );
        let recalled = recall.send_and_wait(question).await?;
        let history = recall.history().page().await?;
        let reads: std::collections::HashMap<_, _> = history
            .messages
            .iter()
            .flat_map(|message| &message.content)
            .filter_map(|part| match part {
                everruns::ContentPart::ToolCall(call) => Some((call.id.as_str(), call)),
                _ => None,
            })
            .collect();
        let memory_reads = history
            .messages
            .iter()
            .flat_map(|message| &message.content)
            .filter(|part| match part {
                everruns::ContentPart::ToolResult(result) if result.error.is_none() => {
                    reads.get(result.tool_call_id.as_str()).is_some_and(|call| {
                        result.result.as_ref().is_some_and(|value| {
                            reads_learned_memory(&call.name, &call.arguments, value, &case.required)
                        })
                    })
                }
                _ => false,
            })
            .count();
        let recall_tool_names: Vec<_> = history
            .messages
            .iter()
            .flat_map(|message| &message.content)
            .filter_map(|part| match part {
                everruns::ContentPart::ToolCall(call) => Some(call.name.as_str()),
                _ => None,
            })
            .collect();
        let memory_ok = case
            .required
            .iter()
            .all(|s| content.to_lowercase().contains(&s.to_lowercase()));
        let lower = recalled.response.to_lowercase();
        let memory_safe = case
            .forbidden_memory
            .iter()
            .all(|s| !content.to_lowercase().contains(&s.to_lowercase()));
        let ok = recalled.success
            && memory_reads > 0
            && memory_ok
            && memory_safe
            && case
                .required
                .iter()
                .all(|s| lower.contains(&s.to_lowercase()))
            && case
                .forbidden
                .iter()
                .all(|s| !lower.contains(&s.to_lowercase()));
        if ok {
            passed += 1
        }
        let read_diagnostics = (!ok).then(|| {
            history
                .messages
                .iter()
                .flat_map(|message| &message.content)
                .filter(|part| {
                    matches!(
                        part,
                        everruns::ContentPart::ToolCall(_) | everruns::ContentPart::ToolResult(_)
                    )
                })
                .collect::<Vec<_>>()
        });
        println!(
            "{}",
            serde_json::json!({"case":case.name,"passed":ok,"memory_valid":memory_ok && memory_safe,"recall":recalled.response,"recall_success":recalled.success,"recall_tool_calls":recalled.tool_calls,"recall_memory_reads":memory_reads,"recall_tool_names":recall_tool_names,"read_diagnostics":read_diagnostics,"memory_files":learned})
        );
    }
    println!(
        "{}",
        serde_json::json!({"passed":passed,"total":cases.len(),"model":model,"provider":if openrouter {"openrouter"}else{"openai"},"evaluation":"lexical behavioral checks; not a general memory benchmark"})
    );
    if passed != cases.len() {
        return Err("dreaming evaluation failed".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::reads_learned_memory;
    use serde_json::json;

    #[test]
    fn directory_listing_and_empty_search_cannot_prove_recall() {
        let required = vec!["eu-west-1".into()];
        assert!(!reads_learned_memory(
            "list_directory",
            &json!({}),
            &json!({"path":"/memory/user/learned/eu-west-1.md"}),
            &required
        ));
        assert!(!reads_learned_memory(
            "grep",
            &json!({}),
            &json!({"pattern":"eu-west-1","matches":[]}),
            &required
        ));
    }
    #[test]
    fn content_search_must_return_the_fact_from_learned_memory() {
        let required = vec!["eu-west-1".into()];
        let mut result = json!({"matches":[{"path":"/memory/user/learned/note.md","line":"Current region: eu-west-1"}]});
        assert!(reads_learned_memory("grep", &json!({}), &result, &required));
        result["matches"][0]["path"] = json!("/workspace/note.md");
        assert!(!reads_learned_memory(
            "grep",
            &json!({}),
            &result,
            &required
        ));
    }
    #[test]
    fn reading_only_the_superseded_fact_cannot_prove_the_correction() {
        let required = vec!["eu-west-1".into()];
        let args = json!({"path":"/memory/user/learned/note.md"});
        assert!(!reads_learned_memory(
            "read_file",
            &args,
            &json!({"content":"Old region: us-east-1"}),
            &required
        ));
        assert!(reads_learned_memory(
            "read_file",
            &args,
            &json!({"content":"Current region: eu-west-1"}),
            &required
        ));
    }

    #[test]
    fn batch_read_must_include_the_current_fact_in_a_learned_file() {
        let required = vec!["eu-west-1".into()];
        let mut result = json!({"results":[
            {"path":"/memory/user/learned/old.md","content":"Old region: us-east-1"},
            {"path":"/memory/user/learned/current.md","content":"Current region: eu-west-1"}
        ]});
        assert!(reads_learned_memory(
            "read_many_files",
            &json!({}),
            &result,
            &required
        ));
        result["results"][1]["error"] = json!("read failed");
        assert!(!reads_learned_memory(
            "read_many_files",
            &json!({}),
            &result,
            &required
        ));
        result["results"][1]
            .as_object_mut()
            .unwrap()
            .remove("content");
        assert!(!reads_learned_memory(
            "read_many_files",
            &json!({}),
            &result,
            &required
        ));
    }
}
