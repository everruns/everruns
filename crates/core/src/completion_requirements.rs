//! Deterministic evidence checks for explicitly configured completion requirements.
//! They inspect this turn's durable transcript; they never execute tools or accept
//! an assistant's claim as proof. Continuation uses the host's existing budget.

#[cfg(any(test, feature = "host"))]
use crate::{RuntimeMessage, RuntimeMessageRole};
#[cfg(any(test, feature = "host"))]
use everruns_contracts::typed_id::MessageId;
use serde::Deserialize;
use serde_json::Value;

pub const CAPABILITY_ID: &str = "completion_requirements";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompletionRequirements {
    checks: Vec<Requirement>,
}

#[derive(Debug, Deserialize)]
// Builtins-only consumers validate configuration; the host evaluates evidence.
#[cfg_attr(not(any(test, feature = "host")), allow(dead_code))]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Requirement {
    ToolResult {
        tool: String,
        #[serde(default)]
        arguments: Option<Value>,
        pointer: String,
        equals: Value,
    },
    FinalJson {
        schema: Value,
    },
}

impl CompletionRequirements {
    pub fn parse(config: &Value) -> Result<Self, String> {
        if config.to_string().len() > 65_536 {
            return Err("Completion requirements exceed 64 KiB".into());
        }
        let parsed: Self = serde_json::from_value(config.clone()).map_err(|e| e.to_string())?;
        if parsed.checks.is_empty() || parsed.checks.len() > 32 {
            return Err("Configure between 1 and 32 completion checks".into());
        }
        for check in &parsed.checks {
            match check {
                Requirement::ToolResult { tool, pointer, .. } => {
                    if tool.trim().is_empty() || (!pointer.is_empty() && !pointer.starts_with('/'))
                    {
                        return Err("Tool checks need a tool name and a JSON pointer".into());
                    }
                }
                Requirement::FinalJson { schema } => {
                    jsonschema::validator_for(schema)
                        .map_err(|e| format!("Invalid final JSON schema: {e}"))?;
                }
            }
        }
        Ok(parsed)
    }

    /// Returns the first unmet requirement. The latest matching call decides:
    /// an older passing result cannot hide a later failure or missing result.
    #[cfg(any(test, feature = "host"))]
    pub fn unmet(
        &self,
        messages: &[RuntimeMessage],
        input_id: MessageId,
        final_text: &str,
    ) -> Option<String> {
        let current_turn = messages
            .iter()
            .position(|m| m.id == input_id)
            .map(|start| &messages[start..]);
        for (index, check) in self.checks.iter().enumerate() {
            let satisfied = match check {
                Requirement::ToolResult {
                    tool,
                    arguments,
                    pointer,
                    equals,
                } => {
                    let Some(messages) = current_turn else {
                        return Some(
                            "Cannot verify tool evidence: the current turn's input is unavailable."
                                .into(),
                        );
                    };
                    let call = messages
                        .iter()
                        .enumerate()
                        .rev()
                        .filter(|(_, m)| m.role == RuntimeMessageRole::Agent)
                        .find_map(|(i, m)| {
                            m.tool_calls()
                                .into_iter()
                                .rev()
                                .find(|c| {
                                    c.name == *tool
                                        && arguments.as_ref().is_none_or(|a| a == &c.arguments)
                                })
                                .map(|c| (i, c))
                        });
                    call.is_some_and(|(i, call)| {
                        messages[i + 1..]
                            .iter()
                            .rev()
                            .filter(|m| m.role == RuntimeMessageRole::ToolResult)
                            .filter_map(RuntimeMessage::tool_result_content)
                            .find(|r| r.tool_call_id == call.id)
                            .is_some_and(|r| {
                                r.error.is_none()
                                    && r.result
                                        .as_ref()
                                        .and_then(|v| v.pointer(pointer))
                                        .is_some_and(|v| v == equals)
                            })
                    })
                }
                Requirement::FinalJson { schema } => serde_json::from_str::<Value>(final_text)
                    .ok()
                    .is_some_and(|v| {
                        jsonschema::validator_for(schema)
                            .is_ok_and(|validator| validator.is_valid(&v))
                    }),
            };
            if !satisfied {
                let detail = match check {
                    Requirement::ToolResult { tool, pointer, .. } => {
                        format!("latest matching {tool} result must satisfy {pointer:?}")
                    }
                    Requirement::FinalJson { .. } => {
                        "final answer must be JSON matching the configured schema".into()
                    }
                };
                return Some(format!(
                    "Completion requirement {} is unmet: {detail}. Check the configured requirements before finishing.",
                    index + 1
                ));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::tool_types::ToolCall;
    use serde_json::json;

    fn call(id: &str) -> RuntimeMessage {
        RuntimeMessage::assistant_with_tools(
            "",
            vec![ToolCall {
                id: id.into(),
                name: "verify".into(),
                arguments: json!({}),
            }],
        )
    }

    #[test]
    fn evidence_must_be_current_matching_and_latest() {
        let rules = CompletionRequirements::parse(&json!({"checks":[{"type":"tool_result","tool":"verify","pointer":"/passed","equals":true}]})).unwrap();
        let mut messages = vec![
            call("old"),
            RuntimeMessage::tool_result("old", Some(json!({"passed":true})), None),
            RuntimeMessage::user("new task"),
        ];
        let id = messages[2].id;
        assert!(rules.unmet(&messages, id, "passed").is_some());
        messages.extend([
            call("new"),
            RuntimeMessage::tool_result("new", Some(json!({"passed":true})), None),
        ]);
        assert!(rules.unmet(&messages, id, "done").is_none());
        messages.push(call("latest"));
        assert!(rules.unmet(&messages, id, "done").is_some());
        messages.push(RuntimeMessage::tool_result(
            "latest",
            Some(json!({"passed":false})),
            None,
        ));
        assert!(rules.unmet(&messages, id, "done").is_some());
        assert!(rules.unmet(&messages, MessageId::new(), "done").is_some());
    }

    #[test]
    fn schema_checks_and_invalid_configuration() {
        let rules = CompletionRequirements::parse(&json!({"checks":[{"type":"final_json","schema":{"type":"object","required":["answer"],"properties":{"answer":{"type":"integer"}}}}]})).unwrap();
        let messages = vec![RuntimeMessage::user("answer")];
        assert!(
            rules
                .unmet(&messages, messages[0].id, "{\"answer\":42}")
                .is_none()
        );
        assert!(
            rules
                .unmet(&[], MessageId::new(), "{\"answer\":42}")
                .is_none(),
            "final JSON validation does not need historical evidence"
        );
        for text in ["done", "{}", "{\"answer\":\"42\"}"] {
            assert!(rules.unmet(&messages, messages[0].id, text).is_some());
        }
        for config in [
            json!({"checks":[]}),
            json!({"checks":[{"type":"final_json","schema":{"type":"unknown"}}]}),
            json!({"checks":[{"type":"tool_result","tool":"verify","pointer":"not-pointer","equals":true}]}),
        ] {
            assert!(CompletionRequirements::parse(&config).is_err());
        }
    }
}
