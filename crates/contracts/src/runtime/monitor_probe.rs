//! Scheduled monitors that run their own probe and wake the agent only when
//! something it cares about happens (knowledge/operations/background-limits.md,
//! "Lean scheduled runs").
//!
//! A monitor created with `spawn_background` and a `schedule` names a tool and
//! its arguments. When that tool is one the scheduler can run itself (see
//! [`MONITOR_PROBE_TOOLS`]), each fire runs the tool directly, with no agent
//! turn and no model tokens, and checks the result against the monitor's
//! [`WakeCondition`]. The agent gets a turn only when the condition becomes
//! true.
//!
//! Decision: wakes fire on transitions, not on every matching fire. A poll
//! that waits for `/json/status` to equal `"approved"` wakes the agent once
//! when it does, not again on every later fire that still sees `"approved"`;
//! a `changed` condition wakes once per new value. Without that, a condition
//! that stays true would cost a turn per fire, which is the cost the probe
//! exists to avoid.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Tools the scheduler can run itself as a monitor probe.
///
/// Each runs with the session's network access list and egress routing, the
/// same controls an agent turn applies. Tools that need the session's files,
/// sandbox or connections are not on this list.
pub const MONITOR_PROBE_TOOLS: &[&str] = &["web_fetch", "get_current_time"];

/// Whether the scheduler can run `tool` itself on each monitor fire.
pub fn is_monitor_probe_tool(tool: &str) -> bool {
    MONITOR_PROBE_TOOLS.contains(&tool)
}

/// Key in a monitor task's spec holding its [`WakeCondition`].
pub const WAKE_WHEN_SPEC_KEY: &str = "wake_when";
/// Key in a monitor task's spec holding the [`ProbeState`] of its last fire.
pub const PROBE_STATE_SPEC_KEY: &str = "probe_state";

/// When a probe monitor wakes the agent.
///
/// `path` is a JSON pointer into the probe's observation (see [`observation`]);
/// empty or absent means the whole observation. Exactly one of `equals`,
/// `not_equals` or `changed` is set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WakeCondition {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equals: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_equals: Option<Value>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub changed: bool,
}

impl WakeCondition {
    /// Parse and validate the `wake_when` argument of a tool call.
    pub fn from_tool_arg(value: &Value) -> Result<Self, String> {
        let condition: WakeCondition = serde_json::from_value(value.clone())
            .map_err(|error| format!("wake_when is invalid: {error}"))?;
        if let Some(path) = &condition.path
            && !path.is_empty()
            && !path.starts_with('/')
        {
            return Err(
                "wake_when.path must be a JSON pointer starting with '/', such as /json/status"
                    .to_string(),
            );
        }
        let set = [
            condition.equals.is_some(),
            condition.not_equals.is_some(),
            condition.changed,
        ]
        .into_iter()
        .filter(|set| *set)
        .count();
        if set != 1 {
            return Err(
                "wake_when must set exactly one of equals, not_equals or changed: true".to_string(),
            );
        }
        Ok(condition)
    }

    /// The JSON schema of `wake_when` for tool definitions.
    pub fn tool_schema() -> Value {
        json!({
            "type": "object",
            "description": "Only for a monitor whose tool the scheduler runs itself (web_fetch, get_current_time): each fire runs the tool without waking you, and you get one turn when this condition becomes true. The condition reads the tool result, where a JSON response body is parsed under /json (for example /json/status or /status_code).",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "JSON pointer into the result, such as /json/error. Empty means the whole result."
                },
                "equals": {
                    "description": "Wake when the value at path becomes equal to this."
                },
                "not_equals": {
                    "description": "Wake when the value at path stops being this, for example a pending status."
                },
                "changed": {
                    "type": "boolean",
                    "description": "Wake each time the value at path differs from the previous fire."
                }
            },
            "additionalProperties": false
        })
    }

    /// A short description for messages, such as `/json/error is no longer "pending"`.
    pub fn describe(&self) -> String {
        let path = match self.path.as_deref() {
            Some(path) if !path.is_empty() => path,
            _ => "the result",
        };
        if let Some(value) = &self.equals {
            format!("{path} equals {value}")
        } else if let Some(value) = &self.not_equals {
            format!("{path} is no longer {value}")
        } else {
            format!("{path} changes")
        }
    }

    /// The value this condition reads from an observation; missing is null.
    pub fn observe(&self, observation: &Value) -> Value {
        match self.path.as_deref() {
            Some(path) if !path.is_empty() => {
                observation.pointer(path).cloned().unwrap_or(Value::Null)
            }
            _ => observation.clone(),
        }
    }

    /// Decide whether this fire wakes the agent, given the value observed now
    /// and the state the previous fire left. Returns the state to keep.
    pub fn evaluate(&self, observed: &Value, previous: Option<&ProbeState>) -> (bool, ProbeState) {
        let fingerprint = fingerprint(observed);
        let matched = if let Some(expected) = &self.equals {
            observed == expected
        } else if let Some(unwanted) = &self.not_equals {
            observed != unwanted
        } else {
            previous.is_some_and(|previous| previous.fingerprint != fingerprint)
        };
        let wake = if self.changed {
            matched
        } else {
            matched && !previous.is_some_and(|previous| previous.matched)
        };
        (
            wake,
            ProbeState {
                fingerprint,
                matched,
            },
        )
    }
}

/// What a probe monitor remembers between fires: a fingerprint of the value
/// its condition read, and whether the condition held.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeState {
    pub fingerprint: String,
    pub matched: bool,
}

/// The document a wake condition reads for one probe result.
///
/// A successful result is used as is, plus `json`: the parsed response body
/// when `body` (a raw request) or `content` (a page fetch) holds JSON. A
/// failed tool call is `{"error": message}`, so a condition such as
/// `not_equals` on a pending status also notices that the probe broke.
pub fn observation(result: Result<&Value, &str>) -> Value {
    let result = match result {
        Ok(result) => result,
        Err(error) => return json!({ "error": error }),
    };
    let mut document = result.clone();
    let parsed = ["body", "content"]
        .iter()
        .filter_map(|field| result.get(*field).and_then(Value::as_str))
        .find_map(|text| serde_json::from_str::<Value>(text.trim()).ok())
        .filter(|parsed| parsed.is_object() || parsed.is_array());
    if let (Some(parsed), Some(object)) = (parsed, document.as_object_mut()) {
        object.insert("json".to_string(), parsed);
    }
    document
}

fn fingerprint(value: &Value) -> String {
    let digest = Sha256::digest(value.to_string().as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn condition(value: Value) -> WakeCondition {
        WakeCondition::from_tool_arg(&value).unwrap()
    }

    #[test]
    fn a_condition_needs_exactly_one_test_and_a_pointer_path() {
        assert!(WakeCondition::from_tool_arg(&json!({"path": "/json/error"})).is_err());
        assert!(
            WakeCondition::from_tool_arg(&json!({"equals": 1, "changed": true})).is_err(),
            "two tests"
        );
        assert!(WakeCondition::from_tool_arg(&json!({"path": "json", "changed": true})).is_err());
        assert!(WakeCondition::from_tool_arg(&json!({"changed": true, "other": 1})).is_err());
        assert!(WakeCondition::from_tool_arg(&json!({"path": "", "changed": true})).is_ok());
    }

    #[test]
    fn a_json_body_is_readable_under_json() {
        let raw = json!({"status_code": 400, "body": "{\"error\": \"authorization_pending\"}"});
        let observed = observation(Ok(&raw));
        assert_eq!(observed["json"]["error"], "authorization_pending");
        assert_eq!(observed["status_code"], 400);

        let page = json!({"status_code": 200, "content": "Hello"});
        assert!(observation(Ok(&page)).get("json").is_none());

        assert_eq!(observation(Err("timed out")), json!({"error": "timed out"}));
    }

    #[test]
    fn not_equals_wakes_once_when_a_pending_status_ends() {
        let wake = condition(json!({"path": "/json/error", "not_equals": "authorization_pending"}));
        let pending = observation(Ok(
            &json!({"body": "{\"error\": \"authorization_pending\"}"}),
        ));
        let approved = observation(Ok(&json!({"body": "{\"assertion\": \"a1\"}"})));

        let (woke, state) = wake.evaluate(&wake.observe(&pending), None);
        assert!(!woke);
        let (woke, state) = wake.evaluate(&wake.observe(&pending), Some(&state));
        assert!(!woke);
        let (woke, state) = wake.evaluate(&wake.observe(&approved), Some(&state));
        assert!(woke, "the status ended");
        let (woke, _) = wake.evaluate(&wake.observe(&approved), Some(&state));
        assert!(!woke, "still ended: no second turn");
    }

    #[test]
    fn equals_wakes_on_the_first_fire_when_already_true() {
        let wake = condition(json!({"path": "/status_code", "equals": 200}));
        let (woke, _) = wake.evaluate(&json!(200), None);
        assert!(woke);
    }

    #[test]
    fn changed_wakes_once_per_new_value_and_never_on_the_first_fire() {
        let wake = condition(json!({"changed": true}));
        let (woke, state) = wake.evaluate(&json!("v1"), None);
        assert!(!woke, "the first fire is the baseline");
        let (woke, state) = wake.evaluate(&json!("v1"), Some(&state));
        assert!(!woke);
        let (woke, state) = wake.evaluate(&json!("v2"), Some(&state));
        assert!(woke);
        let (woke, _) = wake.evaluate(&json!("v2"), Some(&state));
        assert!(!woke);
    }

    #[test]
    fn descriptions_read_as_plain_conditions() {
        assert_eq!(
            condition(json!({"path": "/json/error", "not_equals": "pending"})).describe(),
            "/json/error is no longer \"pending\""
        );
        assert_eq!(
            condition(json!({"changed": true})).describe(),
            "the result changes"
        );
    }
}
