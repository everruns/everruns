//! Deterministic, sample-aware scorers for the `platform` capability study.
//!
//! Tool names alone are too weak: every platform operation goes through one of
//! `discover`, `query`, or `execute`. The command scorer therefore inspects the
//! arguments on `tool.started` events, while the scheduled-agent scorer checks
//! the resources persisted by the server after the turn.

use mira::scorer::scorer;

use crate::friction::{CallClass, Friction, Surface};
use mira::{Sample, Score, Scorer, Transcript};
use regex::Regex;
use serde_json::Value;

pub fn expected_tools() -> Box<dyn Scorer> {
    scorer("expected_tools", |sample: &Sample, t: &Transcript| {
        let Some(expect) = sample
            .metadata
            .get("expect_tools")
            .and_then(Value::as_array)
        else {
            return Score::na("expected_tools", "sample declares no expected tools");
        };
        let missing = expect
            .iter()
            .filter_map(|entry| {
                let tool = entry.get("tool")?.as_str()?;
                let min = entry.get("min").and_then(Value::as_u64).unwrap_or(1) as usize;
                // Roles, not raw names: see `platform_calls`.
                let count = roles(t).iter().filter(|role| role == &tool).count();
                (count < min).then(|| format!("{tool} ({count}/{min})"))
            })
            .collect::<Vec<_>>();
        if missing.is_empty() {
            Score::pass("expected_tools", format!("saw {:?}", roles(t)))
        } else {
            Score::fail(
                "expected_tools",
                format!("missing {}; saw {:?}", missing.join(", "), roles(t)),
            )
        }
    })
}

pub fn forbidden_tools() -> Box<dyn Scorer> {
    scorer("forbidden_tools", |sample: &Sample, t: &Transcript| {
        let Some(forbid) = sample
            .metadata
            .get("forbid_tools")
            .and_then(Value::as_array)
        else {
            return Score::na("forbidden_tools", "sample declares no forbidden tools");
        };
        let hit = roles(t)
            .into_iter()
            .filter(|call| forbid.iter().any(|value| value.as_str() == Some(call)))
            .collect::<Vec<_>>();
        if hit.is_empty() {
            Score::pass("forbidden_tools", "no forbidden tool was called")
        } else {
            Score::fail(
                "forbidden_tools",
                format!("called forbidden tools: {hit:?}"),
            )
        }
    })
}

/// Ensure the model asks before creating the reusable org-wide Agent and does
/// not execute mutations until the second user turn confirms them.
pub fn confirmation_boundary() -> Box<dyn Scorer> {
    scorer(
        "confirmation_boundary",
        |sample: &Sample, t: &Transcript| {
            if sample
                .metadata
                .get("expect_confirmation")
                .and_then(Value::as_bool)
                != Some(true)
            {
                return Score::na(
                    "confirmation_boundary",
                    "sample declares no confirmation boundary",
                );
            }
            let Some(second_input) = t
                .events
                .iter()
                .enumerate()
                .filter(|(_, event)| {
                    event.get("type").and_then(Value::as_str) == Some("input.message")
                })
                .nth(1)
                .map(|(index, _)| index)
            else {
                return Score::fail(
                    "confirmation_boundary",
                    "second confirmation turn was not recorded",
                );
            };
            let before_confirmation = &t.events[..second_input];
            let executed_early = platform_calls(before_confirmation)
                .iter()
                .any(|(tool, _)| tool == "execute");
            let asked = before_confirmation
                .iter()
                .rev()
                .filter(|event| {
                    event.get("type").and_then(Value::as_str) == Some("output.message.completed")
                })
                .filter_map(event_message_text)
                .any(|text| requests_confirmation(&text));
            if executed_early {
                Score::fail(
                    "confirmation_boundary",
                    "called execute before user confirmation",
                )
            } else if !asked {
                Score::fail(
                    "confirmation_boundary",
                    "first response did not request confirmation",
                )
            } else {
                Score::pass(
                    "confirmation_boundary",
                    "asked for confirmation before execute",
                )
            }
        },
    )
}

/// Check command names/arguments carried by Platform `tool.started` events.
///
/// Metadata: `expect_commands` / `forbid_commands` entries contain `tool` and
/// `regex`; expected entries may also set `min` (default 1).
pub fn platform_commands() -> Box<dyn Scorer> {
    scorer("platform_commands", |sample: &Sample, t: &Transcript| {
        let expected = sample
            .metadata
            .get("expect_commands")
            .and_then(Value::as_array);
        let forbidden = sample
            .metadata
            .get("forbid_commands")
            .and_then(Value::as_array);
        if expected.is_none() && forbidden.is_none() {
            return Score::na(
                "platform_commands",
                "sample declares no command expectations",
            );
        }

        let calls = platform_calls(&t.events);
        let mut failures = Vec::new();
        for entry in expected.into_iter().flatten() {
            let tool = entry.get("tool").and_then(Value::as_str).unwrap_or("");
            let pattern = entry.get("regex").and_then(Value::as_str).unwrap_or("");
            let min = entry.get("min").and_then(Value::as_u64).unwrap_or(1) as usize;
            match Regex::new(pattern) {
                Ok(re) => {
                    let count = calls
                        .iter()
                        .filter(|(name, args)| name == tool && re.is_match(args))
                        .count();
                    if count < min {
                        failures.push(format!("missing {tool} /{pattern}/ ({count}/{min})"));
                    }
                }
                Err(error) => failures.push(format!("invalid expected regex /{pattern}/: {error}")),
            }
        }
        for entry in forbidden.into_iter().flatten() {
            let tool = entry.get("tool").and_then(Value::as_str).unwrap_or("");
            let pattern = entry.get("regex").and_then(Value::as_str).unwrap_or("");
            match Regex::new(pattern) {
                Ok(re)
                    if calls
                        .iter()
                        .any(|(name, args)| name == tool && re.is_match(args)) =>
                {
                    failures.push(format!("called forbidden {tool} /{pattern}/"));
                }
                Ok(_) => {}
                Err(error) => {
                    failures.push(format!("invalid forbidden regex /{pattern}/: {error}"))
                }
            }
        }

        if failures.is_empty() {
            Score::pass(
                "platform_commands",
                format!("command sequence matched: {calls:?}"),
            )
        } else {
            Score::fail(
                "platform_commands",
                format!("{}; saw {calls:?}", failures.join("; ")),
            )
        }
    })
}

pub fn tool_budget() -> Box<dyn Scorer> {
    scorer("tool_budget", |sample: &Sample, t: &Transcript| {
        let Some(max) = sample
            .metadata
            .get("max_tool_calls")
            .and_then(Value::as_u64)
        else {
            return Score::na("tool_budget", "sample declares no tool budget");
        };
        let max_iterations = sample
            .metadata
            .get("max_iterations")
            .and_then(Value::as_u64);
        let tools_ok = t.tool_calls_count <= max as usize;
        let iterations_ok = max_iterations.is_none_or(|limit| t.iterations <= limit as usize);
        if tools_ok && iterations_ok {
            Score::pass(
                "tool_budget",
                format!(
                    "{} tool calls <= {max}; {} iterations <= {}",
                    t.tool_calls_count,
                    t.iterations,
                    max_iterations.map_or("unbounded".to_string(), |limit| limit.to_string())
                ),
            )
        } else {
            Score::fail(
                "tool_budget",
                format!(
                    "{} tool calls (max {max}); {} iterations (max {})",
                    t.tool_calls_count,
                    t.iterations,
                    max_iterations.map_or("unbounded".to_string(), |limit| limit.to_string())
                ),
            )
        }
    })
}

/// The `everruns` command tree, as the friction count reads it.
///
/// A rejection is the tree's own wording: clap's `error:` lines (unknown flag,
/// missing argument, invalid value), the tree's `unknown command` answer to a
/// wrong noun or verb, and the shell's `command not found` for a guessed flat
/// name the shell surface does not have.
pub fn cli_surface() -> Surface {
    Surface {
        command: "everruns",
        is_group: crate::control_plane::is_help_node,
        discovery_verbs: &[],
        rejection_prefixes: &["error:"],
        rejection_markers: &["unknown command `", "command not found"],
    }
}

/// Classify a transcript's `bash` calls against the command tree.
pub fn friction_of(t: &Transcript) -> Friction {
    crate::friction::analyze(&t.events, &cli_surface())
}

/// Record the friction breakdown as transcript metrics, so a Mira host's JSON
/// and CSV exports carry `friction.help_reads`, `friction.rejected`,
/// `friction.real` and `friction.other` per run. Nothing is recorded for a run
/// with no `bash` calls (the legacy arm), so its metrics read as unreported
/// rather than as zero friction.
pub fn record_friction(t: &mut Transcript) {
    let friction = friction_of(t);
    if friction.is_empty() {
        return;
    }
    for (name, value) in friction.metrics() {
        t.record_metric(name, value);
    }
}

/// How a run's shell calls were spent: help reads, rejected guesses, and calls
/// that ran a real command.
///
/// Informational, never failing: the value is the share of command-line calls
/// that did real work, so it lowers a case's aggregate as friction rises
/// without turning a run that finished into a failure. Budgets stay with
/// `tool_budget`. N/A on a run with no `bash` calls.
pub fn cli_friction() -> Box<dyn Scorer> {
    scorer("cli_friction", |_sample: &Sample, t: &Transcript| {
        let friction = friction_of(t);
        if friction.is_empty() {
            return Score::na("cli_friction", "no bash calls");
        }
        let cli = friction.calls.len() - friction.count(CallClass::Other);
        let share = if cli == 0 {
            1.0
        } else {
            friction.count(CallClass::Real) as f64 / cli as f64
        };
        Score::graded("cli_friction", share, 0.0, friction.summary())
    })
}

pub fn response_matches() -> Box<dyn Scorer> {
    scorer("response_matches", |sample: &Sample, t: &Transcript| {
        let expected = sample.metadata.get("expect_regex").and_then(Value::as_str);
        let forbidden = sample
            .metadata
            .get("forbid_response_regex")
            .and_then(Value::as_str);
        if expected.is_none() && forbidden.is_none() {
            return Score::na(
                "response_matches",
                "sample declares no response expectation",
            );
        }
        let mut failures = Vec::new();
        if let Some(pattern) = expected {
            match Regex::new(pattern) {
                Ok(re) if !re.is_match(&t.final_response) => {
                    failures.push(format!("did not match /{pattern}/"))
                }
                Err(error) => failures.push(format!("invalid regex /{pattern}/: {error}")),
                _ => {}
            }
        }
        if let Some(pattern) = forbidden {
            match Regex::new(pattern) {
                Ok(re) if re.is_match(&t.final_response) => {
                    failures.push(format!("matched forbidden /{pattern}/"))
                }
                Err(error) => {
                    failures.push(format!("invalid forbidden regex /{pattern}/: {error}"))
                }
                _ => {}
            }
        }
        if failures.is_empty() {
            Score::pass("response_matches", "response constraints matched")
        } else {
            Score::fail("response_matches", failures.join("; "))
        }
    })
}

/// Verify the exact autonomous-agent outcome, including the model reference and
/// the absence of a schedule attached to the Platform Chat session itself.
pub fn scheduled_agent_state() -> Box<dyn Scorer> {
    scorer(
        "scheduled_agent_state",
        |sample: &Sample, t: &Transcript| {
            let Some(expect) = sample.metadata.get("expect_scheduled_agent") else {
                return Score::na(
                    "scheduled_agent_state",
                    "sample declares no scheduled agent expectation",
                );
            };
            let Some(state) = t.metadata.get("platform_state") else {
                return Score::fail(
                    "scheduled_agent_state",
                    "subject captured no platform state",
                );
            };
            let mut failures = Vec::new();
            if let Some(error) = state.get("error").and_then(Value::as_str) {
                failures.push(error.to_string());
            }
            let expected_name = t
                .metadata
                .get("resource_name")
                .and_then(Value::as_str)
                .unwrap_or("");
            let agent = state.get("agent").unwrap_or(&Value::Null);
            if agent.get("name").and_then(Value::as_str) != Some(expected_name) {
                failures.push(format!("agent {expected_name:?} was not persisted"));
            }
            let expected_model = expect.get("model_id").and_then(Value::as_str).unwrap_or("");
            let model = state.get("model").unwrap_or(&Value::Null);
            if model.get("model_id").and_then(Value::as_str) != Some(expected_model) {
                failures.push(format!("default model is not {expected_model}"));
            }
            let agent_id = agent.get("id").and_then(Value::as_str);
            let expected_cron = expect
                .get("cron_expression")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let expected_cadence = expect.get("cron_cadence").and_then(Value::as_str);
            let expected_message = expect
                .get("message_regex")
                .and_then(Value::as_str)
                .unwrap_or("");
            let message_re = Regex::new(expected_message).ok();
            let trigger_matches =
                state
                    .get("triggers")
                    .and_then(Value::as_array)
                    .is_some_and(|items| {
                        items.iter().any(|trigger| {
                            trigger.get("agent_id").and_then(Value::as_str) == agent_id
                                && trigger.get("enabled").and_then(Value::as_bool) == Some(true)
                                && trigger
                                    .pointer("/config/cron_expression")
                                    .and_then(Value::as_str)
                                    .is_some_and(|actual| {
                                        cron_matches(expected_cron, expected_cadence, actual)
                                    })
                                && message_re.as_ref().is_some_and(|re| {
                                    trigger
                                        .pointer("/config/message")
                                        .and_then(Value::as_str)
                                        .is_some_and(|message| re.is_match(message))
                                })
                        })
                    });
            if !trigger_matches {
                failures.push(format!(
                    "no enabled {:?} agent trigger with the expected message",
                    expected_cadence.unwrap_or(expected_cron)
                ));
            }
            if let Some(expected_mcp) = expect.get("mcp") {
                let expected_url = expected_mcp
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let expected_name = format!("{expected_name}-visti");
                let server = state
                    .pointer("/mcp_servers/data")
                    .and_then(Value::as_array)
                    .and_then(|items| {
                        items.iter().find(|item| {
                            item.get("name").and_then(Value::as_str) == Some(&expected_name)
                        })
                    });
                match server {
                    Some(server)
                        if server.get("url").and_then(Value::as_str) == Some(expected_url)
                            && server.get("auth_mode").and_then(Value::as_str) == Some("none") =>
                    {
                        let server_id = server.get("id").and_then(Value::as_str).map(normalized_id);
                        let attached = agent
                            .get("capabilities")
                            .and_then(Value::as_array)
                            .is_some_and(|caps| {
                                caps.iter().any(|cap| {
                                    cap.get("ref")
                                        .and_then(Value::as_str)
                                        .map(normalized_id)
                                        .is_some_and(|id| Some(id) == server_id)
                                })
                            });
                        if !attached {
                            failures.push(
                                "Visti MCP server was not attached to the worker agent".to_string(),
                            );
                        }
                    }
                    _ => failures.push(
                        "Visti MCP server was not registered without inline authentication"
                            .to_string(),
                    ),
                }
                let binding_exists = state
                    .pointer("/credentials/data")
                    .and_then(Value::as_array)
                    .is_some_and(|items| {
                        items.iter().any(|binding| {
                            binding.get("tool_name").and_then(Value::as_str) == Some("visti_send")
                                && binding.get("parameter_name").and_then(Value::as_str)
                                    == Some("channel_key")
                                && binding.get("configured").and_then(Value::as_bool) == Some(false)
                                && binding.get("setup_url").and_then(Value::as_str).is_some()
                                && binding.get("value").is_none()
                        })
                    });
                if !binding_exists {
                    failures
                        .push("pending Visti Agent credential setup was not created".to_string());
                }
            }
            if state
                .get("session_schedules")
                .and_then(Value::as_array)
                .is_some_and(|items| !items.is_empty())
            {
                failures.push(
                    "created a Platform Chat session schedule instead of only an agent trigger"
                        .to_string(),
                );
            }
            if let Some(error) = state
                .pointer("/session_schedules/error")
                .and_then(Value::as_str)
            {
                failures.push(error.to_string());
            }

            if failures.is_empty() {
                Score::pass(
                    "scheduled_agent_state",
                    format!("persisted {expected_name} with {expected_model} and hourly trigger"),
                )
            } else {
                Score::fail("scheduled_agent_state", failures.join("; "))
            }
        },
    )
}

fn normalized_id(value: &str) -> String {
    value
        .trim_start_matches("mcp:")
        .trim_start_matches("mcp_")
        .chars()
        .filter(|ch| ch.is_ascii_hexdigit())
        .collect()
}

/// The role each tool call played, in order.
fn roles(t: &Transcript) -> Vec<String> {
    platform_calls(&t.events)
        .into_iter()
        .map(|(role, _)| role)
        .collect()
}

fn platform_calls(events: &[Value]) -> Vec<(String, String)> {
    events
        .iter()
        .filter(|event| event.get("type").and_then(Value::as_str) == Some("tool.started"))
        .filter_map(|event| {
            let call = event.pointer("/data/tool_call")?;
            let arguments = call.get("arguments").unwrap_or(&Value::Null);
            // The serialized arguments *and* the script inside them. A pattern
            // like `\blist_harnesses\b` fails against the serialization alone
            // when the command follows a newline, because `\n` serializes to
            // the two characters `\` and `n` and `n` is a word character, so
            // the word boundary never matches. Appending the raw script makes
            // every command in a multi-line script anchorable.
            let script = arguments
                .get("commands")
                .and_then(Value::as_str)
                .unwrap_or_default();
            // The dataset names v1's tools. On the shell arm there is one
            // tool, so the role a call plays is read off its script; see
            // `control_plane::script_mutates`.
            let name = call.get("name")?.as_str()?;
            let role = match name {
                "bash" if crate::control_plane::script_mutates(script) => "execute",
                "bash" => "query",
                other => other,
            };
            Some((
                role.to_string(),
                format!("{}\n{script}", serde_json::to_string(arguments).ok()?),
            ))
        })
        .collect()
}

fn event_message_text(event: &Value) -> Option<String> {
    event
        .pointer("/data/message/content")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
}

fn requests_confirmation(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("confirm")
        || (lower.contains('?')
            && ["proceed", "continue", "go ahead"]
                .iter()
                .any(|phrase| lower.contains(phrase)))
}

fn cron_matches(expected: &str, cadence: Option<&str>, actual: &str) -> bool {
    match cadence {
        None => actual == expected,
        Some("hourly") => matches!(
            actual.split_whitespace().collect::<Vec<_>>().as_slice(),
            ["0", "*", "*", "*", "*"] | ["0", "0", "*", "*", "*", "*", "*"]
        ),
        Some(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample(metadata: Value) -> Sample {
        serde_json::from_value(json!({"id":"test","input":["test"],"metadata":metadata})).unwrap()
    }

    #[tokio::test]
    async fn command_scorer_reads_platform_arguments() {
        let sample = sample(json!({
            "expect_commands":[{"tool":"execute","regex":"create_agent_trigger"}],
            "forbid_commands":[{"tool":"execute","regex":"create_session_schedule"}]
        }));
        let transcript = Transcript {
            events: vec![
                json!({"type":"tool.started","data":{"tool_call":{"name":"execute","arguments":{"commands":"create_agent_trigger --agent_id agent_1"}}}}),
            ],
            ..Default::default()
        };
        assert!(platform_commands().score(&sample, &transcript).await.pass);
    }

    #[tokio::test]
    async fn scheduled_agent_scorer_resolves_model_and_trigger_state() {
        let sample = sample(json!({"expect_scheduled_agent":{
            "model_id":"gpt-5.6-terra", "cron_cadence":"hourly", "message_regex":"(?i)dad joke"
        }}));
        let mut transcript = Transcript::default();
        transcript
            .metadata
            .insert("resource_name".into(), json!("eval-joke-123"));
        transcript.metadata.insert("platform_state".into(), json!({
            "agent":{"id":"agent_1","name":"eval-joke-123","default_model_id":"model_1"},
            "model":{"id":"model_1","model_id":"gpt-5.6-terra"},
            "triggers":[{"agent_id":"agent_1","enabled":true,"config":{"cron_expression":"0 0 * * * * *","message":"Tell me a dad joke"}}],
            "session_schedules":[]
        }));
        assert!(
            scheduled_agent_state()
                .score(&sample, &transcript)
                .await
                .pass
        );
    }

    #[tokio::test]
    async fn confirmation_scorer_rejects_execute_before_second_input() {
        let sample = sample(json!({"expect_confirmation":true}));
        let transcript = Transcript {
            events: vec![
                json!({"type":"input.message"}),
                json!({"type":"tool.started","data":{"tool_call":{"name":"execute","arguments":{"commands":"create_agent"}}}}),
                json!({"type":"output.message.completed","data":{"message":{"content":[{"type":"text","text":"Please confirm."}]}}}),
                json!({"type":"input.message"}),
            ],
            ..Default::default()
        };
        assert!(
            !confirmation_boundary()
                .score(&sample, &transcript)
                .await
                .pass
        );
    }

    #[tokio::test]
    async fn confirmation_scorer_ignores_later_empty_assistant_messages() {
        let sample = sample(json!({"expect_confirmation":true}));
        let transcript = Transcript {
            events: vec![
                json!({"type":"input.message"}),
                json!({"type":"output.message.completed","data":{"message":{"content":[{"type":"text","text":"Do you want me to proceed? Please confirm."}]}}}),
                json!({"type":"output.message.completed","data":{"message":{"content":[]}}}),
                json!({"type":"input.message"}),
            ],
            ..Default::default()
        };
        assert!(
            confirmation_boundary()
                .score(&sample, &transcript)
                .await
                .pass
        );
    }

    #[test]
    fn confirmation_accepts_a_question_asking_to_proceed() {
        assert!(requests_confirmation(
            "This creates an organization-wide Agent. Shall I proceed?"
        ));
        assert!(!requests_confirmation("I will proceed now."));
    }

    /// A recorded shell-arm run covering every class. The tool results are in
    /// the live server's shape (content parts, one holding the bash tool's
    /// serialized `{stdout, stderr}`), so this pins the reader the live subject
    /// relies on, not only the offline one.
    const FRICTION_FIXTURE: &str = include_str!("../fixtures/friction-shell-run.json");

    fn fixture_transcript() -> Transcript {
        let fixture: Value = serde_json::from_str(FRICTION_FIXTURE).unwrap();
        Transcript {
            events: fixture["events"].as_array().unwrap().clone(),
            ..Default::default()
        }
    }

    #[test]
    fn friction_classifies_every_call_in_a_recorded_run() {
        let friction = friction_of(&fixture_transcript());
        let classes: Vec<&str> = friction.calls.iter().map(|c| c.class.name()).collect();
        assert_eq!(
            classes,
            [
                "help", "help", "rejected", "rejected", "rejected", "real", "other"
            ],
            "the read_file call is not a shell call and is not counted"
        );
        assert_eq!(
            friction.summary(),
            "7 calls: 2 help, 3 rejected, 1 real, 1 other"
        );
        // The refusal text comes from the stderr the bash tool serialized.
        assert_eq!(
            friction.calls[2].error.as_deref(),
            Some("error: unexpected argument '--limit' found")
        );
        assert!(
            friction.calls[3]
                .error
                .as_deref()
                .is_some_and(|e| e.contains("list_agent_triggers: command not found"))
        );
    }

    #[test]
    fn friction_report_lists_commands_errors_and_help_pages_in_order() {
        let report = friction_of(&fixture_transcript()).report("everruns");
        assert_eq!(report["help_reads"], 2);
        assert_eq!(report["rejected"], 3);
        assert_eq!(report["real"], 1);
        assert_eq!(
            report["help_pages"],
            json!([
                "everruns agents",
                "everruns skills list",
                "everruns agents triggers"
            ])
        );
        let commands = report["commands"].as_array().unwrap();
        let line = |text: &str| {
            commands
                .iter()
                .find(|c| c["command"].as_str().is_some_and(|c| c.starts_with(text)))
                .unwrap_or_else(|| panic!("no {text} in {commands:?}"))
        };
        assert_eq!(line("everruns skills list --limit")["outcome"], "rejected");
        assert_eq!(
            line("everruns skills list --limit")["command"],
            "everruns skills list --limit 5",
            "the pipeline's filter is not part of the command"
        );
        // A guessed name the shell has never heard of is still a command tried.
        assert_eq!(line("list_agent_triggers")["kind"], "unknown");
        assert_eq!(line("list_agent_triggers")["outcome"], "rejected");
        // In a call that read help and then guessed, the guess is the culprit.
        assert_eq!(line("everruns agents triggers --help")["outcome"], "help");
        let guess = line("everruns agents triggers show");
        assert_eq!(guess["outcome"], "rejected");
        assert!(
            guess["error"]
                .as_str()
                .unwrap()
                .starts_with("unknown command `show`"),
            "{guess}"
        );
        // Every invocation in a real call is listed, substitutions included.
        assert_eq!(
            commands
                .iter()
                .filter(|c| c["call"] == 5 && c["outcome"] == "ok")
                .count(),
            3,
            "{commands:?}"
        );
    }

    /// The same classes from the outputs the offline control plane really
    /// produces, so a change to the tree's wording that the classifier no
    /// longer recognises fails here rather than silently counting as real.
    #[tokio::test]
    async fn friction_recognises_the_trees_own_refusals() {
        let plane = crate::control_plane::FakeControlPlane::new(
            crate::control_plane::Harness::PlatformChat,
        );
        let cases = [
            ("everruns --help", "help"),
            ("everruns agents", "help"),
            ("everruns agents triggers", "help"),
            ("everruns agents create -h", "help"),
            ("everruns skills list --limit 20", "rejected"),
            ("everruns agents frobnicate", "rejected"),
            ("everruns agentz list", "rejected"),
            ("list_agents", "rejected"),
            ("everruns agents list", "real"),
            ("everruns agents list --help; everruns agents list", "real"),
            ("echo hi", "other"),
        ];
        for (script, expected) in cases {
            let output = plane.call("bash", &json!({ "commands": script })).await;
            let call = crate::friction::classify(script, &output, &cli_surface());
            assert_eq!(call.class.name(), expected, "{script}: {output}");
        }
    }

    #[tokio::test]
    async fn friction_scorer_is_informational_and_na_without_shell_calls() {
        let sample = sample(json!({}));
        let score = cli_friction().score(&sample, &fixture_transcript()).await;
        assert!(score.pass && !score.na, "{score:?}");
        // One real call out of six command-line calls.
        assert!((score.value - 1.0 / 6.0).abs() < 1e-9, "{score:?}");

        let legacy = Transcript {
            events: vec![
                json!({"type":"tool.started","data":{"tool_call":{"name":"query","arguments":{"commands":"list_agents"}}}}),
            ],
            ..Default::default()
        };
        assert!(cli_friction().score(&sample, &legacy).await.na);
        let mut legacy = legacy;
        record_friction(&mut legacy);
        assert!(legacy.metrics.is_empty(), "no shell calls, no metrics");

        let mut shell = fixture_transcript();
        record_friction(&mut shell);
        assert_eq!(shell.metric("friction.help_reads"), Some(2.0));
        assert_eq!(shell.metric("friction.rejected"), Some(3.0));
        assert_eq!(shell.metric("friction.real"), Some(1.0));
    }

    #[test]
    fn hourly_cadence_accepts_supported_cron_forms() {
        assert!(cron_matches("", Some("hourly"), "0 * * * *"));
        assert!(cron_matches("", Some("hourly"), "0 0 * * * * *"));
        assert!(!cron_matches("", Some("hourly"), "0 0 * * *"));
    }
}
